use std::{
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use crossbeam_channel::Receiver;
use pixels::{Error, Pixels, SurfaceTexture};

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha20Rng;
use tao::{
    dpi::PhysicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::{Window, WindowBuilder},
};

const WIDTH: u32 = 512;
const HEIGHT: u32 = 512;
const TILE_SIZE: u32 = 32;
const N_TILES_X: u32 = ((WIDTH as f32 + (TILE_SIZE - 1) as f32) / TILE_SIZE as f32) as u32;
const N_TILES_Y: u32 = ((HEIGHT as f32 + (TILE_SIZE - 1) as f32) / TILE_SIZE as f32) as u32;
const N_TILES: u32 = N_TILES_X * N_TILES_Y;
const N_THREADS: usize = 4;
const COLOR_CHANNELS: usize = 4;

fn main() -> Result<(), Error> {
    env_logger::init();

    let event_loop = EventLoop::new();

    let size = PhysicalSize::new(WIDTH, HEIGHT);

    let window = Arc::new(
        WindowBuilder::new()
            .with_title("A fantastic window!")
            .with_inner_size(size)
            .with_resizable(false)
            .build(&event_loop)
            .unwrap(),
    );

    let pixels_mutex = {
        let surface_texture = SurfaceTexture::new(WIDTH, HEIGHT, &window);
        Arc::new(Mutex::new(Pixels::new(WIDTH, WIDTH, surface_texture)?))
    };

    run_draw_threads(Arc::clone(&pixels_mutex), Arc::clone(&window));

    event_loop.run(move |event, _, control_flow| {
        //println!("{:?}", event);

        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => *control_flow = ControlFlow::Exit,
                _ => (),
            },
            Event::RedrawRequested(_) => {
                let pixels = pixels_mutex.lock().unwrap();
                if let Err(err) = pixels.render() {
                    println!("pixels.render: {}", err);
                    *control_flow = ControlFlow::Exit;
                }
            }
            _ => (),
        }
    })
}

fn run_draw_threads(pixels_mutex: Arc<Mutex<Pixels>>, window: Arc<Window>) {
    let (tx, rx) = crossbeam_channel::bounded(N_THREADS);

    for _thread in 0..N_THREADS {
        run_draw_thread(rx.clone(), Arc::clone(&pixels_mutex));
    }

    thread::spawn(move || loop {
        for tile_idx in 0..N_TILES {
            tx.send(tile_idx).unwrap();
        }

        window.request_redraw();
    });
}

fn run_draw_thread(tile_rx: Receiver<u32>, pixels_mutex: Arc<Mutex<Pixels>>) {
    thread::spawn(move || {
        for tile_idx in tile_rx.iter() {
            let mut pixels = pixels_mutex.lock().unwrap();
            let frame = pixels.frame_mut();

            let (min_x, min_y, max_x, max_y) = get_tile_bounds(tile_idx);

            let mut rng = ChaCha20Rng::from_entropy();

            let r: u8 = rng.gen_range(0..255);
            let g: u8 = rng.gen_range(0..255);
            let b: u8 = rng.gen_range(0..255);

            for y in min_y..max_y {
                for x in min_x..max_x {
                    let o = (y * WIDTH + x) as usize * COLOR_CHANNELS;
                    frame[o + 0] = r;
                    frame[o + 1] = g;
                    frame[o + 2] = b;
                    frame[o + 3] = 255;
                }
            }
        }
    });
}

fn get_tile_bounds(tile_idx: u32) -> (u32, u32, u32, u32) {
    let tile_x = tile_idx % N_TILES_X;
    let tile_y = tile_idx / N_TILES_X;

    let min_y = tile_y * TILE_SIZE;
    let mut max_y = min_y + TILE_SIZE;
    if max_y > HEIGHT - 1 {
        max_y = HEIGHT - 1;
    }

    let min_x = tile_x * TILE_SIZE;
    let mut max_x = min_x + TILE_SIZE;
    if max_x > WIDTH - 1 {
        max_x = WIDTH - 1;
    }

    (min_x, min_y, max_x, max_y)
}
