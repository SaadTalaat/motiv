use super::{channel, Stream, StreamReceiver, StreamSender};
use super::{RecvError, SendError, TryRecvError};
use std::cell::Cell;
use std::sync::Arc;
/// # Ventilators
///
/// the ventilator distributes messages between child -worker-
/// streams in unicast manner unlike a publisher which
/// broadcasts messages to sub-streams

pub struct Ventilator<T, U>
where
    T: Send,
    U: Send,
{
    streams: Vec<Stream<T, U>>,
    counter: Cell<usize>,
    size: usize,
}

impl<T, U> Ventilator<T, U>
where
    T: Send + 'static,
    U: Send + 'static,
{
    pub fn new<F>(work_fn: F, size: usize) -> Self
    where
        F: Fn(T) -> U + Send + Clone + 'static,
    {
        let mut streams = vec![];
        for _ in 0..size {
            let stream = Stream::new(work_fn.clone());
            streams.push(stream)
        }

        Self {
            size: streams.len(),
            streams,
            counter: Cell::new(0),
        }
    }

    pub fn push(&self, msg: T) -> Result<(), SendError<T>> {
        let stream = &self.streams[self.counter.get() % self.size];
        stream.push(msg)
    }

    pub fn stop(&self) {
        for stream in &self.streams {
            stream.stop();
        }
    }

    // XXX: implement this
    //pub fn join(&self) {
    //    for stream in self.streams.iter() {
    //        stream.join().unwrap();
    //    }
    //}
    pub fn to_streams(&self) -> &[Stream<T, U>] {
        &self.streams
    }
}
