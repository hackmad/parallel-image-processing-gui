use std::{
    cell::RefCell,
    sync::{Arc, LazyLock, Mutex},
    thread,
    time::Duration,
};

use clap::Parser;

use pixels::{Error, Pixels, SurfaceTexture};

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha20Rng;

use tao::{
    dpi::LogicalSize,
    event::{ElementState, Event, KeyEvent, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::Key,
    window::{Window, WindowBuilder},
};

mod app_config;
use app_config::*;

mod threadpool;
use threadpool::*;

static CONFIG: LazyLock<AppConfig> = LazyLock::new(|| AppConfig::parse());

/// Entry point for the application.
fn main() -> Result<(), Error> {
    println!("Running with {} threads", CONFIG.threads());

    // Initialize logger for tao.
    env_logger::init();

    // Create a new event loop for the application.
    let event_loop = EventLoop::new();

    // Create a new window.
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("A fantastic window!")
            .with_inner_size(LogicalSize::new(CONFIG.width.get(), CONFIG.height.get()))
            .with_resizable(false)
            .build(&event_loop)
            .unwrap(),
    );
    let inner_size = window.inner_size();

    // Create a surface texture that uses the logical inner size to render to the entire window's inner dimensions.
    // Then create pixel frame buffer that matches rendered image dimensions.
    let pixels = {
        let surface_texture = SurfaceTexture::new(inner_size.width, inner_size.height, &window);

        Arc::new(Mutex::new(Pixels::new(
            CONFIG.width.get(),
            CONFIG.height.get(),
            surface_texture,
        )?))
    };

    // Create a thread pool for rendering tiles in parallel.
    let pool = Arc::new(Mutex::new(ThreadPool::build(CONFIG.threads()).unwrap()));

    // Track remaining tiles. It will be used to shutdown the thread pool.
    let remaining_tiles = Arc::new(Mutex::new(CONFIG.tiles()));

    // Start a separate thread that will queue all tiles. This way we can run the event loop in main thread.
    {
        let pool = Arc::clone(&pool);
        let pixels = Arc::clone(&pixels);
        let window = Arc::clone(&window);
        thread::spawn(|| render(pool, pixels, window, remaining_tiles));
    }

    // Run the event loop.
    event_loop.run(move |event, _, control_flow| {
        //println!("{:?}", event);
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent { event, .. } => match event {
                // When window is closed or destroyed or Escape key is pressed, stop rendering.
                WindowEvent::CloseRequested
                | WindowEvent::Destroyed
                | WindowEvent::KeyboardInput {
                    event:
                        KeyEvent {
                            logical_key: Key::Escape,
                            state: ElementState::Released,
                            ..
                        },
                    ..
                } => {
                    println!("Exiting application.");
                    pool.lock().unwrap().shutdown();
                    *control_flow = ControlFlow::Exit;
                }
                _ => (),
            },
            Event::RedrawRequested(_) => {
                // Draw the pixel frame buffer to the window. If there are errors show the error and stop rendering.
                if let Err(err) = pixels.lock().map(|p| p.render()) {
                    println!("pixels.render() failed with error.\n{}", err);
                    pool.lock().unwrap().shutdown();
                    *control_flow = ControlFlow::Exit;
                }
            }
            _ => (),
        }
    })
}

/// Use a threadpool to queue up all the tiles for rendering.
fn render(
    pool: Arc<Mutex<ThreadPool>>,
    pixels: Arc<Mutex<Pixels>>,
    window: Arc<Window>,
    remaining_tiles: Arc<Mutex<u32>>,
) {
    // Queue up the tiles to render.
    for tile_idx in 0..CONFIG.tiles() {
        let remaining_tiles = Arc::clone(&remaining_tiles);
        let pixels = Arc::clone(&pixels);
        let window = Arc::clone(&window);

        pool.lock().unwrap().execute(move || {
            thread_local! {
                // Allocate pixels for rendering a tile per thread so we don't allocate for each tile.
                pub static TILE_PIXELS: RefCell<Vec<u8>> = {
                    println!("Allocating tile pixels for {:?}", thread::current().id());
                    RefCell::new(vec![0_u8; CONFIG.tiles_pixel_bytes()])
                };
            }

            TILE_PIXELS.with_borrow_mut(|tile_pixels| {
                render_tile(tile_idx, tile_pixels);
                copy_tile(tile_idx, tile_pixels, pixels);
            });

            *remaining_tiles.lock().unwrap() -= 1;

            window.request_redraw();
        });
    }

    println!("Queued up all tiles to render.");

    // Wait for render to complete and shutdown pool.
    loop {
        if *remaining_tiles.lock().unwrap() == 0 {
            pool.lock().unwrap().shutdown();
            break;
        }

        thread::sleep(Duration::from_secs(1));
    }
}

/// Render a single tile adding some random load to simulate rendering algorithm.
fn render_tile(tile_idx: u32, tile_pixels: &mut [u8]) {
    let mut rng = ChaCha20Rng::seed_from_u64(tile_idx as u64);

    let r: u8 = rng.gen_range(0..255);
    let g: u8 = rng.gen_range(0..255);
    let b: u8 = rng.gen_range(0..255);

    for pixel in tile_pixels.chunks_mut(COLOR_CHANNELS) {
        pixel[0] = r;
        pixel[1] = g;
        pixel[2] = b;
        pixel[3] = 255;
    }

    // Random load.
    thread::sleep(Duration::from_millis(
        rng.gen_range(1..CONFIG.max_load_millis.get()),
    ));
}

/// Copy rendered tile to pixel frame buffer.
fn copy_tile(tile_idx: u32, tile_pixels: &[u8], pixels: Arc<Mutex<Pixels>>) {
    let (x_min, y_min, x_max, y_max) = get_tile_bounds(tile_idx);

    let mut pixels = pixels.lock().unwrap();
    let frame = pixels.frame_mut();

    let w = CONFIG.width.get();
    let ts = CONFIG.tile_size.get() as usize;
    let tw = (x_max - x_min + 1) as usize;

    for y in y_min..=y_max {
        let dst_start = (y * w + x_min) as usize * COLOR_CHANNELS;
        let dst_end = dst_start + tw * COLOR_CHANNELS;

        let src_start = (y - y_min) as usize * ts * COLOR_CHANNELS;
        let src_end = src_start + tw * COLOR_CHANNELS;

        let dst = &mut frame[dst_start..dst_end];
        let src = &tile_pixels[src_start..src_end];
        dst.copy_from_slice(src);
    }
}

/// Return the mininmum and maximum x/y coordinates for a tile index.
fn get_tile_bounds(tile_idx: u32) -> (u32, u32, u32, u32) {
    let tile_x = (tile_idx % CONFIG.tiles_x()) as u32;
    let tile_y = (tile_idx / CONFIG.tiles_x()) as u32;

    let y_min = tile_y * CONFIG.tile_size.get() as u32;
    let mut y_max = y_min as u32 + CONFIG.tile_size.get() as u32 - 1;
    if y_max > CONFIG.height.get() - 1 {
        y_max = CONFIG.height.get() - 1;
    }

    let x_min = tile_x * CONFIG.tile_size.get() as u32;
    let mut x_max = x_min + CONFIG.tile_size.get() as u32 - 1;
    if x_max > CONFIG.width.get() - 1 {
        x_max = CONFIG.width.get() - 1;
    }

    (x_min, y_min, x_max, y_max)
}
