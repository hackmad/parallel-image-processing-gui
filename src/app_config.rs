use clap::Parser;
use std::thread;

pub const COLOR_CHANNELS: usize = 4;

/// Program configuration.
#[derive(Parser, Clone)]
#[command(author, version, about, long_about = None)]
pub struct AppConfig {
    /// Image height.
    #[arg(
        long = "width",
        value_name = "WIDTH",
        default_value_t = 512,
        help = "image width in pixels"
    )]
    pub width: u32,

    /// Image width.
    #[arg(
        long = "height",
        value_name = "HEIGHT",
        default_value_t = 512,
        help = "image height in pixels"
    )]
    pub height: u32,

    /// Number of threads.
    #[arg(
        long = "threads",
        value_name = "THREADS",
        default_value_t = get_max_threads(),
        help = "number of threads to use (default = max logical cores)",
    )]
    num_threads: usize,

    /// Tile size.
    #[arg(
        long = "tile-size",
        value_name = "TILE_SIZE",
        default_value_t = 32,
        help = "tile size in pixels (default = 32)"
    )]
    pub tile_size: u8,

    /// How often to request redraw.
    #[arg(
        long = "redraw-millis",
        short = 'r',
        value_name = "REDRAW_MILLIS",
        default_value_t = 1,
        help = "how often to request redraw in milliseconds (default = 1)"
    )]
    pub redraw_millis: u64,

    /// How often to request redraw.
    #[arg(
        long = "random-load-millis",
        short = 'l',
        value_name = "RANDOM_LOAD_MILLIS",
        default_value_t = 100,
        help = "max time in milliseconds to use to simulate tile rendering load (default = 100)"
    )]
    pub random_load_millis: u64,
}

impl AppConfig {
    /// Returns the number of threads to use.
    pub fn threads(&self) -> usize {
        let max_threads = get_max_threads();
        if self.num_threads == 0 {
            panic!("Invalid num threads");
        } else if self.num_threads > max_threads {
            panic!("Num threads > max logical CPUs {}", max_threads);
        }
        self.num_threads
    }

    pub fn tiles_x(&self) -> u32 {
        ((self.width as f32 + (self.tile_size - 1) as f32) / self.tile_size as f32) as u32
    }

    pub fn tiles_y(&self) -> u32 {
        ((self.height as f32 + (self.tile_size - 1) as f32) / self.tile_size as f32) as u32
    }

    pub fn tiles(&self) -> u32 {
        self.tiles_x() * self.tiles_y()
    }

    pub fn tiles_pixel_bytes(&self) -> usize {
        self.tile_size as usize * self.tile_size as usize * COLOR_CHANNELS
    }
}

/// Returns the number of threads available. If unable, then 1 is returned.
fn get_max_threads() -> usize {
    thread::available_parallelism().map_or(1, |n| n.get())
}
