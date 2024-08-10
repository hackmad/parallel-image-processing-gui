//! Threadpool
//!
//! See https://doc.rust-lang.org/book/ch20-02-multithreaded.html

use std::{
    sync::{mpsc, Arc, Mutex},
    thread,
};

/// Simple thread pool that can execute jobs in fixed number of workers.
pub struct ThreadPool {
    /// List of workers.
    workers: Vec<Worker>,

    /// Used to send a job to workers on an MPSC channel.
    sender: Option<mpsc::Sender<Job>>,

    /// Indicates that the thread pool is shutting down.
    is_shutting_down: bool,
}

impl ThreadPool {
    /// Build a new thread pool of specified size. Returns a `PoolCreationError` if pool size is
    /// zero.
    pub fn build(size: usize) -> Result<Self, PoolCreationError> {
        if size == 0 {
            return Err(PoolCreationError::ZeroPoolSize);
        }

        // Create an MPSC channel to send / receive jobs.
        let (sender, receiver) = mpsc::channel();
        let receiver = Arc::new(Mutex::new(receiver));

        // Allocate workers.
        let mut workers = Vec::with_capacity(size);
        for id in 0..size {
            workers.push(Worker::new(id, Arc::clone(&receiver)));
        }

        Ok(Self {
            workers,
            sender: Some(sender),
            is_shutting_down: false,
        })
    }

    /// Execute a function in a worker thread as long as pool is not shut down.
    pub fn execute<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        if !self.is_shutting_down {
            let job = Box::new(f);
            self.sender.as_ref().unwrap().send(job).unwrap();
        }
    }

    /// Shut down the pool.
    pub fn shutdown(&mut self) {
        if !self.is_shutting_down {
            // Set flag to shutdown so this won't run more than once.
            self.is_shutting_down = true;

            // Explicitly drop the sender before waiting for the threads to finish.
            drop(self.sender.take());

            // Wait for threads to complete.
            for worker in &mut self.workers {
                println!("Waiting for worker {} to shutdown.", worker.id);
                if let Some(thread) = worker.thread.take() {
                    thread.join().unwrap();
                }
            }
        }
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// A job is a function that runs once.
type Job = Box<dyn FnOnce() + Send + 'static>;

/// Worker thread.
struct Worker {
    id: usize,
    thread: Option<thread::JoinHandle<()>>,
}

impl Worker {
    /// Create a new worker and listen for jobs to execute.
    fn new(id: usize, receiver: Arc<Mutex<mpsc::Receiver<Job>>>) -> Worker {
        let thread = thread::spawn(move || loop {
            let message = receiver.lock().unwrap().recv();

            match message {
                Ok(job) => {
                    println!("Worker {id} got a job; executing.");
                    job();
                }
                Err(_) => {
                    println!("Worker {id} disconnected; shutting down.");
                    break;
                }
            }
        });

        Worker {
            id,
            thread: Some(thread),
        }
    }
}

/// Custom errors for thread pool creation.
#[derive(Debug)]
pub enum PoolCreationError {
    /// Pool size of 0 requested which is useless.
    ZeroPoolSize,
}
