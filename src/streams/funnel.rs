use super::{channel, Stream, StreamMsg, StreamReceiver, StreamSender};
// Re-importing errors from `::super` to allow easy portability
// between `std::sync::mpsc::channel` and `crossbeam_channel`
use super::{RecvError, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

struct FunnelBackend<U>
where
    U: Send,
{
    inputs: Vec<StreamReceiver<U>>,
    output: StreamSender<U>,
    size: usize,
}

impl<U> FunnelBackend<U>
where
    U: Send + 'static,
{
    fn new(inputs: Vec<StreamReceiver<U>>, output: StreamSender<U>) -> Self {
        Self {
            size: inputs.len(),
            inputs,
            output,
        }
    }

    fn run<F>(&self, combine_fn: F)
    where
        F: Fn(Vec<U>) -> U + 'static,
    {
        let mut die_count = 0;
        loop {
            let msgs: Vec<U> = self
                .inputs
                .iter()
                .map(|rx| rx.try_recv())
                .map(|result| match result {
                    Ok(StreamMsg::Frame(data)) => Some(data),
                    Ok(StreamMsg::Die) => {
                        die_count += 1;
                        None
                    }
                    Err(TryRecvError::Empty) => None,
                    // A stream will disconnect after a it sends
                    // a `StreamMsg::Die` msg.
                    // assert disconnected count = die_count
                    Err(TryRecvError::Disconnected) => None,
                })
                .flatten()
                .collect();
            if msgs.len() > 0 {
                let combined = combine_fn(msgs);
                self.output.send(StreamMsg::Frame(combined));
            } else {
                // Yield CPU
                thread::sleep(Duration::from_micros(100))
            }

            if die_count == self.size {
                break;
            }
        }
    }
}
pub struct Funnel<U>
where
    U: Send,
{
    output: StreamReceiver<U>,
    handle: JoinHandle<()>,
}

impl<U> Funnel<U>
where
    U: Send + 'static,
{
    pub fn new<T, F>(streams: &[Stream<T, U>], combine_fn: F) -> Self
    where
        T: Send + 'static,
        F: Fn(Vec<U>) -> U + Send + 'static,
    {
        let receivers = streams.iter().map(|s| s.output.clone()).collect();
        let (btx, brx) = channel();
        let handle = thread::spawn({
            let backend = FunnelBackend::new(receivers, btx);
            move || backend.run(combine_fn)
        });
        Self {
            output: brx,
            handle,
        }
    }

    pub fn many<T, F>(streams: &[Stream<T, U>], combine_fn: F, size: usize) -> Self
    where
        T: Send + 'static,
        F: Fn(Vec<U>) -> U + Send + Clone + 'static,
    {
        if size > streams.len() {
            panic!("Cannot start more funnels than streams");
        }
        // Start child funnels
        let mut funnels = vec![];
        let stream_count = streams.len();
        for i in 0..size {
            // Distribute streams between funnels
            // each funnel gets nStreams % nFunnels or less
            let start_idx = (stream_count % size) * i;
            let end_idx = ((stream_count % size) * (i + 1)).min(stream_count);
            let receivers = &streams[start_idx..=end_idx];
            funnels.push(Self::new(receivers, combine_fn.clone()));
        }
        // Start frontend collector funnel
        let (btx, brx) = channel();
        let handle = thread::spawn({
            let outputs = funnels.iter().map(|f| f.output.clone()).collect();
            let backend = FunnelBackend::new(outputs, btx);
            move || backend.run(combine_fn)
        });
        Self {
            output: brx,
            handle,
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

    pub fn join(self) -> thread::Result<()> {
        self.handle.join()
    }
}

impl<U> Iterator for Funnel<U>
where
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
