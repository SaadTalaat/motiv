use crossbeam_channel::{unbounded, Receiver, RecvError, SendError, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::Duration;
// Re-exports
mod funnel;
mod ventilator;
pub use funnel::Funnel;
pub use ventilator::Ventilator;

#[derive(Debug, PartialEq, Copy, Clone)]
enum StreamMsg<T>
where
    T: Send,
{
    Frame(T),
    Die,
}
type StreamSender<T> = Sender<StreamMsg<T>>;
type StreamReceiver<T> = Receiver<StreamMsg<T>>;

fn channel<T>() -> (StreamSender<T>, StreamReceiver<T>)
where
    T: Send,
{
    unbounded()
}

struct StreamBackend<T, U>
where
    T: Send,
    U: Send,
{
    input: StreamReceiver<T>,
    output: StreamSender<U>,
}

/// # StreamBackend vs Stream
///
/// the backend part of the stream is the part that
/// runs in a thread to process incoming messages.
///
/// what the caller gets when calling `Stream::new(...)`
/// is the frontend which communicates to the backend.
impl<T, U> StreamBackend<T, U>
where
    T: Send,
    U: Send,
{
    fn new(input: StreamReceiver<T>, output: StreamSender<U>) -> Self {
        Self { input, output }
    }

    fn run<F>(&self, work_fn: F)
    where
        F: 'static + Fn(T) -> U,
    {
        loop {
            if let Ok(msg) = self.input.try_recv() {
                match msg {
                    StreamMsg::Frame(t) => {
                        let res = StreamMsg::Frame(work_fn(t));
                        // If output channel has been closed prior
                        // stopping this stream, it should panic
                        // and kill current thread to cascade
                        // failure
                        self.output.send(res).unwrap();
                    }
                    StreamMsg::Die => {
                        self.output.send(StreamMsg::Die).unwrap();
                        break;
                    }
                }
            } else {
                // Yield CPU
                thread::sleep(Duration::from_micros(100))
            }
        }
    }
}

pub struct Stream<T, U>
where
    T: Send,
    U: Send,
{
    input: StreamSender<T>,
    output: StreamReceiver<U>,
    handle: JoinHandle<()>,
}

/// # Streams
///
/// T, U `'static' lifetime is enforced by thread::spawn
/// No references, no thnx! The stream has to own the data.
///
/// The stream consists of a backend and a frontend.
/// A frontend is what the caller gets when calling `Stream::new()`
impl<T, U> Stream<T, U>
where
    T: Send + 'static,
    U: Send + 'static,
{
    pub fn new<F>(work_fn: F) -> Self
    where
        F: Fn(T) -> U + Send + 'static,
    {
        let (ftx, frx) = channel::<T>();
        let (btx, brx) = channel::<U>();
        let handle = thread::spawn({
            let backend = StreamBackend::new(frx, btx);
            move || backend.run(work_fn)
        });

        Self {
            input: ftx,
            output: brx,
            handle: handle,
        }
    }

    pub fn push(&self, msg: T) -> Result<(), SendError<T>> {
        match self.input.send(StreamMsg::Frame(msg)) {
            Err(SendError(StreamMsg::Frame(m))) => Err(SendError(m)),
            Ok(_) => Ok(()),
            // If it's not Ok nor an Err(SendError(Frame))
            // then it's undefined behavior
            _ => panic!("Failed to send message to stream"),
        }
    }

    pub fn pop(&self) -> Result<U, RecvError> {
        if let StreamMsg::Frame(msg) = self.output.recv()? {
            Ok(msg)
        } else {
            Err(RecvError)
        }
    }

    pub fn pop_nowait(&self) -> Result<U, TryRecvError> {
        if let StreamMsg::Frame(msg) = self.output.try_recv()? {
            Ok(msg)
        } else {
            Err(TryRecvError::Empty)
        }
    }

    pub fn stop(&self) {
        self.input.send(StreamMsg::Die).unwrap();
    }

    pub fn join(self) -> thread::Result<()> {
        self.handle.join()
    }
}

impl<T, U> Iterator for Stream<T, U>
where
    T: Send,
    U: Send,
{
    type Item = U;
    fn next(&mut self) -> Option<Self::Item> {
        match self.output.recv() {
            Ok(StreamMsg::Frame(msg)) => Some(msg),
            _ => None,
        }
    }
}
