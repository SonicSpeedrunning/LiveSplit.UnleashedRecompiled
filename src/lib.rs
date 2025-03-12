#![no_std]
#![warn(
    clippy::complexity,
    clippy::correctness,
    clippy::perf,
    clippy::style,
    clippy::undocumented_unsafe_blocks,
    rust_2018_idioms
)]

use asr::{
    future::{next_tick, retry},
    settings::Gui,
    time::Duration,
    timer::{self, TimerState},
    watcher::Watcher,
    Address, FromEndian, Process,
};

mod client_layer;

asr::panic_handler!();
asr::async_main!(stable);

async fn main() {
    let mut settings = Settings::register();
    asr::set_tick_rate(120.0);

    loop {
        // Hook to the target process
        let (process_name, process) = hook_process().await;

        process
            .until_closes(async {
                // Once the target has been found and attached to, set up some default watchers
                let mut watchers = Watchers::default();

                // Perform memory scanning to look for the addresses we need
                let memory = Memory::init(&process, process_name).await;

                loop {
                    // Splitting logic. Adapted from OG LiveSplit:
                    // Order of execution
                    // 1. update() will always be run first. There are no conditions on the execution of this action.
                    // 2. If the timer is currently either running or paused, then the isLoading, gameTime, and reset actions will be run.
                    // 3. If reset does not return true, then the split action will be run.
                    // 4. If the timer is currently not running (and not paused), then the start action will be run.
                    settings.update();
                    update_loop(&process, &memory, &mut watchers);

                    if [TimerState::Running, TimerState::Paused].contains(&timer::state()) {
                        match is_loading(&watchers, &settings) {
                            Some(true) => timer::pause_game_time(),
                            Some(false) => timer::resume_game_time(),
                            _ => (),
                        }

                        match game_time(&watchers, &settings, &memory) {
                            Some(x) => timer::set_game_time(x),
                            _ => (),
                        }

                        match reset(&watchers, &settings) {
                            true => timer::reset(),
                            _ => match split(&watchers, &settings) {
                                true => timer::split(),
                                _ => (),
                            },
                        }
                    }

                    if timer::state().eq(&TimerState::NotRunning) {
                        watchers.igt_buffer = Duration::ZERO;
                        if start(&watchers, &settings) {
                            timer::start();
                            timer::pause_game_time();

                            match is_loading(&watchers, &settings) {
                                Some(true) => timer::pause_game_time(),
                                Some(false) => timer::resume_game_time(),
                                _ => (),
                            }
                        }
                    }

                    next_tick().await;
                }
            })
            .await;
    }
}

#[derive(Gui)]
struct Settings {
    /// Use IGT instead of LRT
    #[default = false]
    igt: bool,
}

#[derive(Default)]
struct Watchers {
    is_loading: Watcher<bool>,
    igt: Watcher<Duration>,
    igt_buffer: Duration,
}

struct Memory {
    base_client_ptr: Address,
}

impl Memory {
    async fn init(game: &Process, _main_module_name: &str) -> Self {
        retry(|| {
            let mut ranges = game.memory_ranges();
            let mut range = ranges.next()?;
            loop {
                if range.size().ok()? == 0x1000 {
                    let val = range.address().ok()?;
                    range = ranges.next()?;

                    if range.size().ok()? == 0xFFFFF000 {
                        return Some(Self {
                            base_client_ptr: val,
                        });
                    }
                } else {
                    range = ranges.next()?;
                }
            }
        })
        .await
    }
}

async fn hook_process() -> (&'static str, Process) {
    retry(|| {
        PROCESS_NAMES
            .iter()
            .find_map(|&name| Some((name, Process::attach(name)?)))
    })
    .await
}

fn update_loop(game: &Process, memory: &Memory, watchers: &mut Watchers) {
    // Loading state represent the current status of the loading screen
    let loading_state = client_layer::read_host_path::<u32>(
        game,
        memory.base_client_ptr,
        &[0x833678A0, 0x4, 0xE0, 0x13C],
    )
    .unwrap_or_default()
    .from_be();

    // This shows whether the game is effectively stuck in a loading state, regardless of the laoding screen shown
    let is_loading =
        client_layer::read_host_path::<u8>(game, memory.base_client_ptr, &[0x83367A4C])
            .map(|val| val != 0)
            .unwrap_or(false);

    watchers
        .is_loading
        .update_infallible(is_loading || (loading_state != 0 && loading_state != 2));

    // We want to store the internal ID of the current level. In reality we are just checking this for the world map (which should return an empty string)
    let stage = client_layer::read_host_path::<u8>(
        game,
        memory.base_client_ptr,
        &[0x83367900, 0x8, 0xAC, 0x0],
    )
    .unwrap_or_default();

    let igt = if stage == 0 {
        Duration::ZERO
    } else {
        client_layer::read_host_path::<f32>(game, memory.base_client_ptr, &[0x83367900, 0x8, 0x5C])
            .map(|val| val.from_be())
            .map(|val| {
                if val.is_nan() || val < 0.0 {
                    Duration::ZERO
                } else {
                    Duration::milliseconds((val * 100.0) as i64 * 10)
                }
            })
            .unwrap_or_default()
    };

    let old_igt = watchers.igt.pair.map(|val| val.current).unwrap_or_default();

    if igt < old_igt {
        watchers.igt_buffer += old_igt;
    }

    watchers.igt.update_infallible(igt);
}

fn start(_watchers: &Watchers, _settings: &Settings) -> bool {
    false
}

fn split(_watchers: &Watchers, _settings: &Settings) -> bool {
    false
}

fn reset(_watchers: &Watchers, _settings: &Settings) -> bool {
    false
}

fn is_loading(watchers: &Watchers, settings: &Settings) -> Option<bool> {
    match settings.igt {
        true => Some(true),
        false => watchers.is_loading.pair.map(|val| val.current),
    }
}

fn game_time(watchers: &Watchers, settings: &Settings, _memory: &Memory) -> Option<Duration> {
    match settings.igt {
        false => None,
        true => watchers
            .igt
            .pair
            .map(|val| val.current + watchers.igt_buffer),
    }
}

const PROCESS_NAMES: &[&str] = &["UnleashedRecomp.exe", "UnleashedRecomp"];
