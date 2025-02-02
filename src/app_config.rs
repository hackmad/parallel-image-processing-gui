//! Application configuration

use clap::Parser;

use std::{
    num::{NonZeroU32, NonZeroU64, NonZeroU8, NonZeroUsize},
    thread,
};

pub const COLOR_CHANNELS: usize = 4;

/// Program configuration.
#[derive(Parser, Clone)]
#[command(author, version, about, long_about = None)]
pub struct AppConfig {
    /// Image height.
    #[arg(
        long = "width",
        value_name = "WIDTH",
        default_value_t = NonZeroU32::new(512).unwrap(),
        help = "image width in pixels",
    )]
    pub width: NonZeroU32,

    /// Image width.
    #[arg(
        long = "height",
        value_name = "HEIGHT",
        default_value_t = NonZeroU32::new(512).unwrap(),
        help = "image height in pixels",
    )]
    pub height: NonZeroU32,

    /// Number of threads.
    #[arg(
        long = "threads",
        value_name = "THREADS",
        default_value_t = NonZeroUsize::new(get_max_threads()).unwrap(),
        help = "number of threads to use (default = max logical cores)",
    )]
    num_threads: NonZeroUsize,

    /// Tile size.
    #[arg(
        long = "tile-size",
        value_name = "TILE_SIZE",
        default_value_t = NonZeroU8::new(32).unwrap(),
        help = "tile size in pixels (default = 32)",
    )]
    pub tile_size: NonZeroU8,

    /// How often to request redraw.
    #[arg(
        long = "max-load-millis",
        short = 'l',
        value_name = "MAX_LOAD_MILLIS",
        default_value_t = NonZeroU64::new(100).unwrap(),
        help = "max time in milliseconds to use to simulate tile rendering load (default = 100)",
    )]
    pub max_load_millis: NonZeroU64,
}

impl AppConfig {
    /// Returns the number of threads to use.
    pub fn threads(&self) -> usize {
        let n_threads = self.num_threads.get();
        if n_threads == 0 {
            panic!("Invalid num threads");
        }

        let max_threads = get_max_threads();
        if n_threads > max_threads {
            panic!("Num threads {n_threads} > max logical CPUs {max_threads}");
        }

        n_threads
    }

    pub fn tiles_x(&self) -> u32 {
        ((self.width.get() as f32 + (self.tile_size.get() - 1) as f32)
            / self.tile_size.get() as f32) as u32
    }

    pub fn tiles_y(&self) -> u32 {
        ((self.height.get() as f32 + (self.tile_size.get() - 1) as f32)
            / self.tile_size.get() as f32) as u32
    }

    pub fn tiles(&self) -> u32 {
        self.tiles_x() * self.tiles_y()
    }

    pub fn tiles_pixel_bytes(&self) -> usize {
        self.tile_size.get() as usize * self.tile_size.get() as usize * COLOR_CHANNELS
    }
}

/// Returns the number of threads available. If unable, then 1 is returned.
fn get_max_threads() -> usize {
    thread::available_parallelism().map_or(1, |n| n.get())
}
