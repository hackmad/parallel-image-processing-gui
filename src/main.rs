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
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::{Window, WindowBuilder},
};

mod app_config;
use app_config::*;

static CONFIG: LazyLock<AppConfig> = LazyLock::new(|| AppConfig::parse());

/// Entry point for the application.
fn main() -> Result<(), Error> {
    env_logger::init();

    let event_loop = EventLoop::new();

    let size = PhysicalSize::new(CONFIG.width, CONFIG.height);

    let window = Arc::new(
        WindowBuilder::new()
            .with_title("A fantastic window!")
            .with_inner_size(size)
            .with_resizable(false)
            .build(&event_loop)
            .unwrap(),
    );

    let pixels = {
        let surface_texture = SurfaceTexture::new(CONFIG.width, CONFIG.height, &window);
        Arc::new(Mutex::new(Pixels::new(
            CONFIG.width,
            CONFIG.height,
            surface_texture,
        )?))
    };

    // Use channels to communicate thread termination.
    let (run_tx, run_rx) = crossbeam_channel::bounded(1);
    let (done_tx, done_rx) = crossbeam_channel::bounded(1);

    run_threads(Arc::clone(&pixels), Arc::clone(&window), run_rx, done_tx);

    event_loop.run(move |event, _, control_flow| {
        //println!("{:?}", event);

        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => {
                    println!("Exit application. Waiting for threads to stop...");
                    message_thread(run_tx.clone(), ThreadMessage::Stop);
                    done_rx.recv().unwrap();

                    *control_flow = ControlFlow::Exit;
                }
                _ => (),
            },
            Event::RedrawRequested(_) => {
                let pixels = pixels.lock().unwrap();
                if let Err(err) = pixels.render() {
                    println!("pixels.render: {}", err);

                    println!("Exit application. Waiting for threads to stop...");
                    message_thread(run_tx.clone(), ThreadMessage::Stop);
                    done_rx.recv().unwrap();

                    *control_flow = ControlFlow::Exit;
                }
            }
            _ => (),
        }
    })
}

// This is just a example to simulate a rendering engine that can display results. Generally these systems take a lot
// of time per section of an image. This can be made to go fast if we want by removing extra threads and just redrawing
// as we merge tiles into the pixel frame buffer.
fn run_threads(
    pixels: Arc<Mutex<Pixels>>,
    window: Arc<Window>,
    run_rx: Receiver<ThreadMessage>,
    done_tx: Sender<ThreadMessage>,
) {
    // Channel to queue up more tiles than number of threads there is always work to do.
    let (tile_processor_tx, tile_processor_rx) = crossbeam_channel::bounded(CONFIG.threads() * 8);

    // Channel to queue up limited number of threads to merge rendered tiles. Because each tile takes up memory we
    // don't want this to be huge but enough so if threads complete faster they aren't waiting to send.
    let (tile_pixels_tx, tile_pixels_rx) = crossbeam_channel::bounded(CONFIG.threads() * 2);

    // Channels used to communicate termination.
    let (redraw_tx, redraw_rx) = crossbeam_channel::bounded(1);
    let (queue_tx, queue_rx) = crossbeam_channel::bounded(1);

    // Spawn the worker threads that will wait to render tiles.
    for _thread in 0..CONFIG.threads() {
        let tile_processor_rx = tile_processor_rx.clone();
        let tile_pixels_tx = tile_pixels_tx.clone();
        thread::spawn(move || render_tile(tile_processor_rx, tile_pixels_tx));
    }

    // Drop extra receiver/sender.
    drop(tile_processor_rx);

    // Spawn a thread to periodically redraw the full image.
    thread::spawn(move || redraw_window(window, redraw_rx));

    // Spawn a thread to copy tile to pixel frame buffer.
    thread::spawn(move || copy_tile(pixels, tile_pixels_rx));

    // Spawn a thread to queue up tiles to render in a random order.
    let tile_processor_txc = tile_processor_tx.clone();
    thread::spawn(move || queue_tiles(tile_processor_txc, queue_rx));

    // Spawn a thread that listens for the main thread to signal an exit so it can in turn do the
    // same for the threads spawned here.
    thread::spawn(move || loop {
        match run_rx.try_recv() {
            Ok(_) | Err(TryRecvError::Disconnected) => {
                println!("Terminating run_threads()");
                message_thread(queue_tx, ThreadMessage::Stop);
                message_thread(redraw_tx, ThreadMessage::Stop);
                message_thread(tile_processor_tx, TileMessage::Stop);
                for _thread in 0..CONFIG.threads() {
                    message_thread(tile_pixels_tx.clone(), CopyTileMessage::Stop);
                }
                message_thread(done_tx, ThreadMessage::Stop);

                break;
            }
            Err(TryRecvError::Empty) => {
                thread::sleep(Duration::from_millis(1));
            }
        }
    });
}

fn message_thread<T: Clone>(tx: Sender<T>, message: T) {
    loop {
        match tx.try_send(message.clone()) {
            Ok(_) => break,                              // Sent message over.
            Err(TrySendError::Full(_)) => continue,      // Channel full. Try again.
            Err(TrySendError::Disconnected(_)) => break, // Already terminated.
        }
    }
}

fn redraw_window(window: Arc<Window>, redraw_rx: Receiver<ThreadMessage>) {
    loop {
        match redraw_rx.try_recv() {
            Ok(_) | Err(TryRecvError::Disconnected) => {
                println!("Terminating redraw_window()");
                break;
            }
            Err(TryRecvError::Empty) => {
                window.request_redraw();
                thread::sleep(Duration::from_millis(CONFIG.redraw_millis));
            }
        }
    }
}

fn queue_tiles(tile_processor_tx: Sender<TileMessage>, queue_rx: Receiver<ThreadMessage>) {
    let mut rng = ChaCha20Rng::from_entropy();

    let mut tiles: Vec<_> = (0..CONFIG.tiles()).collect();
    tiles.shuffle(&mut rng);

    for tile_idx in tiles {
        match queue_rx.try_recv() {
            Ok(_) | Err(TryRecvError::Disconnected) => {
                println!("Terminating queue_tiles()");
                break;
            }
            Err(TryRecvError::Empty) => {
                message_thread(tile_processor_tx.clone(), TileMessage::Process(tile_idx));
            }
        }
    }
}

fn render_tile(tile_processor_rx: Receiver<TileMessage>, tile_pixels_tx: Sender<CopyTileMessage>) {
    let mut rng = ChaCha20Rng::from_entropy();

    let mut tile_pixels = vec![0_u8; CONFIG.tiles_pixel_bytes()];

    for message in tile_processor_rx.iter() {
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
                    rng.gen_range(1..CONFIG.random_load_millis),
                ));

                message_thread(
                    tile_pixels_tx.clone(),
                    CopyTileMessage::Merge(tile_idx, tile_pixels.clone()),
                );
            }
            TileMessage::Stop => {
                println!("Terminating render_tile()");
                break;
            }
        }
    }
}

fn copy_tile(pixels: Arc<Mutex<Pixels>>, tile_pixels_rx: Receiver<CopyTileMessage>) {
    for message in tile_pixels_rx.iter() {
        match message {
            CopyTileMessage::Merge(tile_idx, tile_pixels) => {
                let (x_min, y_min, _x_max, y_max) = get_tile_bounds(tile_idx);

                let mut pixels = pixels.lock().unwrap();
                let frame = pixels.frame_mut();

                for y in y_min..y_max {
                    let dst_start = (y * CONFIG.width + x_min) as usize * COLOR_CHANNELS;
                    let dst_end = dst_start + CONFIG.tile_size as usize * COLOR_CHANNELS;

                    let src_start =
                        (y - y_min) as usize * CONFIG.tile_size as usize * COLOR_CHANNELS;
                    let src_end = src_start + CONFIG.tile_size as usize * COLOR_CHANNELS;

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

fn get_tile_bounds(tile_idx: u32) -> (u32, u32, u32, u32) {
    let tile_x = tile_idx % CONFIG.tiles_x();
    let tile_y = tile_idx / CONFIG.tiles_x();

    let min_y = tile_y * CONFIG.tile_size as u32;
    let mut max_y = min_y + CONFIG.tile_size as u32;
    if max_y > CONFIG.height - 1 {
        max_y = CONFIG.height - 1;
    }

    let min_x = tile_x * CONFIG.tile_size as u32;
    let mut max_x = min_x + CONFIG.tile_size as u32;
    if max_x > CONFIG.width - 1 {
        max_x = CONFIG.width - 1;
    }

    (min_x, min_y, max_x, max_y)
}

#[derive(Copy, Clone)]
enum ThreadMessage {
    Stop,
}

#[derive(Copy, Clone)]
enum TileMessage {
    Process(u32),
    Stop,
}

#[derive(Clone)]
enum CopyTileMessage {
    Merge(u32, Vec<u8>),
    Stop,
}
