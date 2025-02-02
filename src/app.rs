//! The winit applicaition

use crate::{
    app_config::COLOR_CHANNELS,
    threadpool::ThreadPool,
    CONFIG,
};

use std::{
    cell::RefCell, 
    sync::{Arc, Mutex, OnceLock},
    thread,
    time::Duration,
};

use pixels::{Pixels, SurfaceTexture};

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha20Rng;

use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalSize},
    error::EventLoopError,
    event::{ElementState, KeyEvent, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
    keyboard::{Key, NamedKey},
    window::Window,
};


/// User events for the render loop.
#[derive(Debug, Clone, PartialEq)]
enum UserEvent{
    // Render the tile.
    RenderTile {
        /// The tile pixel data.
        tile_pixels: Vec<u8>,

        /// The tile index.
        tile_idx: u32,
    },
}

/// This proxy will be used to trigger custom events from the render loop to the winit application window.
static EVENT_LOOP_PROXY: OnceLock<EventLoopProxy<UserEvent>> = OnceLock::new();

/// The winit application.
struct App {
    /// The preview window.
    window: Option<Arc<Window>>,

    /// The preview image pixels.
    pixels: Option<Pixels<'static>>,

    /// The preview image pixel dimensions.
    pixel_size: LogicalSize<u32>,

    /// The inner dimensions of the preview window.
    window_inner_size: PhysicalSize<u32>,
}

impl App {
    /// Render the preview image to the window.
    fn render(&self) -> Result<(), String> {
        self.pixels.as_ref().map_or(Ok(()), |pixels| pixels.render())
            .map_err(|err| format!("{}", err))
    }

    /// Resize the preview image.
    ///
    /// * `pixel_size`        - The dimensions of the preview image.
    /// * `window_inner_size` - The inner dimensions of the preview window.
    fn resize_pixels(
        &mut self,
        pixel_size: LogicalSize<u32>,
        window_inner_size: PhysicalSize<u32>,
    ) -> Result<(), String> {
        // Render only if the application has initialized and we have pixels and window.
        self.pixels.as_mut().map_or(Ok(()), |pixels| {
            // Resize the pixel surface texture to fit the windows inner dimensions.
            match pixels.resize_surface(window_inner_size.width, window_inner_size.height) {
                Ok(()) => {
                    // Resize the pixel image buffer.
                    match pixels.resize_buffer(pixel_size.width, pixel_size.height) {
                        Ok(()) => {
                            // Store the new sizes.
                            self.pixel_size = pixel_size;
                            self.window_inner_size = window_inner_size;

                            // Request a redraw.
                            self.window.as_ref().map(|window| window.request_redraw());
                            Ok(())
                        }
                        Err(err) => Err(format!("pixels.resize_buffer() failed.\n{}", err)),
                    }
                }
                Err(err) => Err(format!("pixels.resize_surface() failed to resize frame buffer surface.\n{}", err)),
            }
        })
    }
}

impl Default for App {
    /// Returns the "default value" for `App` initialized to the default dimensions.
    fn default() -> Self {
        Self {
            window: None,
            pixels: None,
            pixel_size: LogicalSize::new(CONFIG.width.get(), CONFIG.height.get()),
            window_inner_size: PhysicalSize::new(CONFIG.width.get(), CONFIG.height.get()),
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Create a new window.
        let window_attributes = Window::default_attributes()
            .with_title("PBRT v3 (Rust)")
            .with_inner_size(self.window_inner_size)
            .with_resizable(true);

        let window = Arc::new(event_loop.create_window(window_attributes).expect("Unable to create window"));

        // Save the inner dimensions of the preview window.
        let window_inner_size = window.inner_size();

        // Create a surface texture that uses the logical inner size to render to the entire window's inner
        // dimensions.
        let surface_texture = SurfaceTexture::new(
            window_inner_size.width,
            window_inner_size.height,
            Arc::clone(&window),
        );

        // Create pixel frame buffer that matches rendered image dimensions that will be used to display it
        // in the window.
        let pixels = Pixels::new(self.pixel_size.width, self.pixel_size.height, surface_texture)
            .expect("Unable to create pixel frame buffer for window");

        self.window = Some(Arc::clone(&window));
        self.pixels = Some(pixels);
        self.window_inner_size = window_inner_size;
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                println!("The close button was pressed; stopping");
                event_loop.exit();
            }

            WindowEvent::RedrawRequested => {
                match self.render() {
                    Ok(()) => (),
                    Err(err) => {
                        eprintln!("Error redrawing pixels {}", err);
                        event_loop.exit();
                    }
                }
                self.window.as_ref().map(|window| window.request_redraw());
            }
            
            WindowEvent::Resized(new_window_inner_size) => {
                match self.resize_pixels(self.pixel_size, new_window_inner_size) {
                    Ok(()) => (),
                    Err(err) => {
                        eprintln!("Error resizing window {}", err);
                        event_loop.exit();
                    }
                }
            }

            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key: key,
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => match key {
                Key::Named(NamedKey::Escape) => {
                    println!("Escape key was pressed; stopping");
                    event_loop.exit();
                }
                _ => (),
            },

            _ => (),
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) { 
        match event {
            UserEvent::RenderTile { tile_pixels, tile_idx } => {
                self.pixels.as_mut().map(|window_pixels| {
                    copy_tile(tile_idx, &tile_pixels, window_pixels);
                    self.window.as_ref().map(|window| window.request_redraw());
                });
            }
        }
    }
}

/// Run the event loop displaying a window until it is closed or some error occurs.
pub fn run_event_loop() -> Result<(), EventLoopError> {
    eprintln!("Creating event loop");
    let event_loop = EventLoop::<UserEvent>::with_user_event().build().expect("Unable to create event loop");

    eprintln!("Creating event loop proxy");
    EVENT_LOOP_PROXY.get_or_init(|| event_loop.create_proxy());

    eprintln!("Running winit app");
    let mut app = App::default();
    event_loop.run_app(&mut app)
}

/// Send a user event to the event loop.
fn send_user_event(event: UserEvent) {
    // The rendering is done a different thread. We could end up here before the event loop is created. So just 
    // check and wait until event loop is ready. This loop will execute only once when the first scene starts 
    // processing.
    while EVENT_LOOP_PROXY.get().is_none() {
        thread::sleep(Duration::from_millis(100));
    }
    EVENT_LOOP_PROXY.get().map(|proxy| proxy.send_event(event));
}

/// Use a threadpool to queue up all the tiles for rendering.
pub fn render(pool: Arc<Mutex<ThreadPool>>, remaining_tiles: Arc<Mutex<u32>>) {
    // Queue up the tiles to render.
    for tile_idx in 0..CONFIG.tiles() {
        let remaining_tiles = Arc::clone(&remaining_tiles);

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
                send_user_event(UserEvent::RenderTile { tile_pixels: tile_pixels.to_owned(), tile_idx });
            });

            *remaining_tiles.lock().unwrap() -= 1;
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
fn copy_tile(tile_idx: u32, tile_pixels: &[u8], pixels: &mut Pixels) {
    let (x_min, y_min, x_max, y_max) = get_tile_bounds(tile_idx);

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

