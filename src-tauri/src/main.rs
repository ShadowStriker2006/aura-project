#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod advisor;
mod config;
mod credentials;
mod ddragon;
mod game;
mod lcu;
mod live_client;
mod overlay;
mod perf;
mod riotapi;
mod spotify;
mod theme;
mod tray;

use tauri::Manager;

fn main() {
    let runtime_config = config::RuntimeConfig::from_environment();
    runtime_config.log_safe_summary();
    let config_status = runtime_config.status();
    let advisor_state = advisor::AdvisorState::new(runtime_config.advisor.clone())
        .unwrap_or_else(advisor::AdvisorState::fallback);
    let advisor_startup = advisor_state.clone();

    tauri::Builder::default()
        .manage(config::ConfigState::new(config_status))
        .manage(advisor_state)
        .manage(live_client::LiveClientEventState::default())
        .manage(spotify::SpotifyState::new(runtime_config.spotify))
        .manage(spotify::EmbeddedSpotifyState::default())
        .manage(lcu::LocalRiotAccountState::default())
        .manage(ddragon::DDragonCache::default())
        .manage(theme::ThemeState::default())
        .manage(overlay::OverlayState::default())
        .manage(riotapi::RiotApiState::new(runtime_config.riot_api_key))
        .invoke_handler(tauri::generate_handler![
            advisor::advisor_status,
            advisor::advisor_refresh,
            advisor::advisor_draft_mandate,
            advisor::advisor_live_orders,
            advisor::advisor_post_game,
            config::get_integration_config,
            config::save_riot_api_key,
            config::clear_riot_api_key,
            spotify::spotify_login,
            spotify::spotify_play,
            spotify::spotify_pause,
            spotify::spotify_previous,
            spotify::spotify_skip,
            spotify::spotify_playback_status,
            spotify::spotify_devices,
            spotify::spotify_transfer,
            spotify::spotify_open_web_player,
            spotify::embedded::spotify_start_browser_player,
            spotify::embedded::spotify_start_embedded_player,
            spotify::embedded::spotify_stop_embedded_player,
            spotify::embedded::spotify_embedded_status,
            lcu::get_local_riot_account,
            ddragon::get_champion_map,
            ddragon::get_item_map,
            ddragon::get_ddragon_version,
            ddragon::get_champion_image_id_map,
            ddragon::get_champion_details,
            ddragon::get_rune_trees,
            overlay::show_overlay,
            overlay::hide_overlay,
            overlay::overlay_status,
            overlay::get_overlay_layout,
            overlay::set_overlay_layout,
            overlay::toggle_overlay_interaction,
            theme::set_theme,
            theme::get_theme,
            riotapi::set_riot_id,
            riotapi::select_riot_profile,
            riotapi::get_summoner_profile,
            riotapi::get_league_entries,
            riotapi::get_champion_masteries,
            riotapi::fetch_recent_matches,
            riotapi::get_match_detail,
            riotapi::timeline::get_match_timeline,
        ])
        .setup(move |app| {
            tray::init(app.handle())?;

            let handle = app.handle().clone();
            let telemetry_handle = app.handle().clone();
            let ddragon_cache = app.state::<ddragon::DDragonCache>().inner().clone();

            tauri::async_runtime::spawn(perf::process_guard::run_guard_loop(
                "League of Legends.exe",
                10,
            ));
            tauri::async_runtime::spawn(lcu::run_lcu_supervisor(handle));
            tauri::async_runtime::spawn(game::telemetry::run_telemetry_loop(telemetry_handle));
            tauri::async_runtime::spawn(ddragon::run_with_retry(ddragon_cache));
            tauri::async_runtime::spawn(advisor::warm_cache(advisor_startup.clone()));

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("[AURA::FATAL] tauri build failed");
}
