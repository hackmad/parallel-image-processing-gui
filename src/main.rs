use std::{
    sync::{Arc, LazyLock, Mutex},
    thread,
    time::Duration,
};

use clap::Parser;
use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError};

use pixels::{Error, Pixels, SurfaceTexture};

use rand::{seq::SliceRandom, Rng, SeedableRng};
use rand_chacha::ChaCha20Rng;

use tao::{
    dpi::PhysicalSize,
    event::{ElementState, Event, KeyEvent, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::Key,
    window::{Window, WindowBuilder},
};

mod app_config;
use app_config::*;

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
            .with_inner_size(PhysicalSize::new(CONFIG.width.get(), CONFIG.height.get()))
            .with_resizable(false)
            .build(&event_loop)
            .unwrap(),
    );

    // Setup a pixel frame buffer to display image as it is rendered.
    let pixels = {
        let surface_texture = SurfaceTexture::new(CONFIG.width.get(), CONFIG.height.get(), &window);
        Arc::new(Mutex::new(Pixels::new(
            CONFIG.width.get(),
            CONFIG.height.get(),
            surface_texture,
        )?))
    };

    // Use channels to communicate thread termination.
    let (stop_render_tx, stop_render_rx) = crossbeam_channel::bounded(1);
    let (stop_complete_tx, stop_complete_rx) = crossbeam_channel::bounded(1);

    // Start the threads to render the image.
    start_rendering(
        Arc::clone(&pixels),
        Arc::clone(&window),
        stop_render_rx,
        stop_complete_tx.clone(),
    );

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
                    stop_rendering(stop_render_tx.clone(), stop_complete_rx.clone());
                    *control_flow = ControlFlow::Exit;
                }
                _ => (),
            },
            Event::RedrawRequested(_) => {
                // Draw the pixel frame buffer to the window. If there are errors show the error and stop rendering.
                if let Err(err) = pixels.lock().map(|p| p.render()) {
                    println!("pixels.render: {}", err);
                    stop_rendering(stop_render_tx.clone(), stop_complete_rx.clone());
                    *control_flow = ControlFlow::Exit;
                }
            }
            _ => (),
        }
    })
}

/// Send the stop message to the main rendering thread and wait for it to respond with done.
fn stop_rendering(
    stop_render_tx: Sender<ThreadMessage>,
    stop_complete_rx: Receiver<ThreadMessage>,
) {
    println!("Exit application. Waiting for threads to stop...");
    message_thread(stop_render_tx.clone(), ThreadMessage::Stop);
    stop_complete_rx.recv().unwrap();
}

/// This is just a example to simulate a rendering engine that can display results. Generally these systems take a lot
/// of time per section of an image. This can be made to go fast if we want by removing extra threads and just redrawing
/// as we merge tiles into the pixel frame buffer.
fn start_rendering(
    pixels: Arc<Mutex<Pixels>>,
    window: Arc<Window>,
    stop_render_rx: Receiver<ThreadMessage>,
    stop_complete_tx: Sender<ThreadMessage>,
) {
    // Channel to queue up more tiles than number of threads there is always work to do.
    let (process_tile_tx, process_tile_rx) = crossbeam_channel::bounded(CONFIG.threads() * 8);

    // Channel to queue up limited number of threads to merge rendered tiles. Because each tile takes up memory we
    // don't want this to be huge but enough so if threads complete faster they aren't waiting to send.
    let (copy_tile_tx, copy_tile_rx) = crossbeam_channel::bounded(CONFIG.threads() * 2);

    // Channels used to communicate termination.
    let (stop_redraw_tx, stop_redraw_rx) = crossbeam_channel::bounded(1);
    let (stop_queue_tx, stop_queue_rx) = crossbeam_channel::bounded(1);

    // Spawn the worker threads that will wait to render tiles.
    for _thread in 0..CONFIG.threads() {
        let process_tile_rx = process_tile_rx.clone();
        let copy_tile_tx = copy_tile_tx.clone();
        thread::spawn(move || render_tile(process_tile_rx, copy_tile_tx));
    }

    // Drop extra receiver/sender.
    drop(process_tile_rx);

    // Spawn a thread to periodically redraw the full image.
    thread::spawn(move || redraw_window(window, stop_redraw_rx));

    // Spawn a thread to copy tile to pixel frame buffer.
    thread::spawn(move || copy_tile(pixels, copy_tile_rx));

    // Spawn a thread to queue up tiles to render in a random order.
    let process_tile_txc = process_tile_tx.clone();
    thread::spawn(move || queue_tiles(process_tile_txc, stop_queue_rx));

    // Spawn a thread that handles termination.
    thread::spawn(move || {
        wait_for_exit(
            stop_render_rx,
            stop_queue_tx,
            stop_redraw_tx,
            process_tile_tx,
            copy_tile_tx,
            stop_complete_tx,
        )
    });
}

/// Listens for a signal to exit and passes that on to other channels.
fn wait_for_exit(
    stop_render_rx: Receiver<ThreadMessage>,
    stop_queue_tx: Sender<ThreadMessage>,
    stop_redraw_tx: Sender<ThreadMessage>,
    process_tile_tx: Sender<TileMessage>,
    copy_tile_tx: Sender<CopyTileMessage>,
    stop_complete_tx: Sender<ThreadMessage>,
) {
    // Will only ever receive the stop message. So once it is received, start the process to stop.
    for _ in stop_render_rx.iter() {
        println!("Terminating run_threads()");
        message_thread(stop_queue_tx, ThreadMessage::Stop);
        message_thread(stop_redraw_tx, ThreadMessage::Stop);
        message_thread(process_tile_tx, TileMessage::Stop);
        for _thread in 0..CONFIG.threads() {
            message_thread(copy_tile_tx.clone(), CopyTileMessage::Stop);
        }
        message_thread(stop_complete_tx, ThreadMessage::Stop);

        break;
    }
}

/// Sends a message to a thread until it is sent or channel is disconnected.
fn message_thread<T: Clone>(tx: Sender<T>, message: T) {
    loop {
        match tx.try_send(message.clone()) {
            Ok(_) => break,                              // Sent message over.
            Err(TrySendError::Full(_)) => continue,      // Channel full. Try again.
            Err(TrySendError::Disconnected(_)) => break, // Already terminated.
        }
    }
}

/// Send redraw request periodically. If the redraw channel is disconnected it stops.
fn redraw_window(window: Arc<Window>, stop_redraw_rx: Receiver<ThreadMessage>) {
    loop {
        match stop_redraw_rx.try_recv() {
            Ok(_) | Err(TryRecvError::Disconnected) => {
                println!("Terminating redraw_window()");
                break;
            }
            Err(TryRecvError::Empty) => {
                window.request_redraw();
                thread::sleep(Duration::from_millis(CONFIG.redraw_millis.get()));
            }
        }
    }
}

/// Queue messages for the tile processor. If the queue channel is disconnected it stops.
fn queue_tiles(process_tile_tx: Sender<TileMessage>, stop_queue_rx: Receiver<ThreadMessage>) {
    let mut rng = ChaCha20Rng::from_entropy();

    let mut tiles: Vec<_> = (0..CONFIG.tiles()).collect();
    tiles.shuffle(&mut rng);

    for tile_idx in tiles {
        match stop_queue_rx.try_recv() {
            Ok(_) | Err(TryRecvError::Disconnected) => {
                println!("Terminating queue_tiles()");
                break;
            }
            Err(TryRecvError::Empty) => {
                message_thread(
                    process_tile_tx.clone(),
                    TileMessage::Process(Tile(tile_idx)),
                );
            }
        }
    }
}

/// Render tiles until a stop message is received.
fn render_tile(process_tile_rx: Receiver<TileMessage>, copy_tile_tx: Sender<CopyTileMessage>) {
    let mut rng = ChaCha20Rng::from_entropy();

    let mut tile_pixels = vec![0_u8; CONFIG.tiles_pixel_bytes()];

    for message in process_tile_rx.iter() {
        match message {
            TileMessage::Process(tile_idx) => {
                let r: u8 = rng.gen_range(0..255);
                let g: u8 = rng.gen_range(0..255);
                let b: u8 = rng.gen_range(0..255);

                for pixel in tile_pixels.chunks_mut(COLOR_CHANNELS) {
                    pixel[0] = r;
                    pixel[1] = g;
                    pixel[2] = b;
                    pixel[3] = 255;
                }

                thread::sleep(Duration::from_millis(
                    rng.gen_range(1..CONFIG.max_load_millis.get()),
                ));

                message_thread(
                    copy_tile_tx.clone(),
                    CopyTileMessage::Merge(TilePixels::new(tile_idx, tile_pixels.clone())),
                );
            }
            TileMessage::Stop => {
                println!("Terminating render_tile()");
                break;
            }
        }
    }
}

/// Copy rendered tiles to pixel frame buffer until a stop message is received.
fn copy_tile(pixels: Arc<Mutex<Pixels>>, copy_tile_rx: Receiver<CopyTileMessage>) {
    for message in copy_tile_rx.iter() {
        match message {
            CopyTileMessage::Merge(TilePixels {
                tile: Tile(tile_idx),
                pixels: tile_pixels,
            }) => {
                let (x_min, y_min, _x_max, y_max) = get_tile_bounds(tile_idx);

                let mut pixels = pixels.lock().unwrap();
                let frame = pixels.frame_mut();

                for y in y_min..y_max {
                    let dst_start = (y * CONFIG.width.get() + x_min) as usize * COLOR_CHANNELS;
                    let dst_end = dst_start + CONFIG.tile_size.get() as usize * COLOR_CHANNELS;

                    let src_start =
                        (y - y_min) as usize * CONFIG.tile_size.get() as usize * COLOR_CHANNELS;
                    let src_end = src_start + CONFIG.tile_size.get() as usize * COLOR_CHANNELS;

                    let dst = &mut frame[dst_start..dst_end];
                    let src = &tile_pixels[src_start..src_end];
                    dst.copy_from_slice(src);
                }
            }
            CopyTileMessage::Stop => {
                println!("Terminating copy_tile()");
                break;
            }
        }
    }
}

/// Return the mininmum and maximum x/y coordinates for a tile index.
fn get_tile_bounds(tile_idx: u32) -> (u32, u32, u32, u32) {
    let tile_x = tile_idx % CONFIG.tiles_x();
    let tile_y = tile_idx / CONFIG.tiles_x();

    let min_y = tile_y * CONFIG.tile_size.get() as u32;
    let mut max_y = min_y + CONFIG.tile_size.get() as u32;
    if max_y > CONFIG.height.get() - 1 {
        max_y = CONFIG.height.get() - 1;
    }

    let min_x = tile_x * CONFIG.tile_size.get() as u32;
    let mut max_x = min_x + CONFIG.tile_size.get() as u32;
    if max_x > CONFIG.width.get() - 1 {
        max_x = CONFIG.width.get() - 1;
    }

    (min_x, min_y, max_x, max_y)
}

/// Generic messages for a thread.
#[derive(Copy, Clone)]
enum ThreadMessage {
    Stop,
}

/// Messages for tile processor.
#[derive(Copy, Clone)]
enum TileMessage {
    Process(Tile),
    Stop,
}

/// Messages for thread that copies tile to pixel frame.
#[derive(Clone)]
enum CopyTileMessage {
    Merge(TilePixels),
    Stop,
}

/// Tile index.
#[derive(Copy, Clone)]
struct Tile(u32);

/// Tile pixel data.
#[derive(Clone)]
struct TilePixels {
    /// Tile index.
    tile: Tile,

    /// Pixel data.
    pixels: Vec<u8>,
}

impl TilePixels {
    /// Create new tile.
    fn new(tile: Tile, pixels: Vec<u8>) -> Self {
        TilePixels { tile, pixels }
    }
}
