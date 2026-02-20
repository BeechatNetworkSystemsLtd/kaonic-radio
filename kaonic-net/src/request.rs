use crate::{error::NetworkError, network_time_elapsed, packet::PacketId, request, NetworkTime};
use core::marker::PhantomData;

pub trait Responder<T> {
    fn respond(self, id: PacketId, response: T);
}

pub struct Request<T, R: Responder<T>> {
    id: PacketId,
    start_time: NetworkTime,
    timeout: core::time::Duration,
    responder: R,
    _response_type: PhantomData<T>,
}

impl<T, R: Responder<T>> Request<T, R> {
    pub fn new(
        id: PacketId,
        current_time: NetworkTime,
        timeout: core::time::Duration,
        responder: R,
    ) -> Self {
        Self {
            id,
            start_time: current_time,
            timeout,
            responder,
            _response_type: PhantomData::default(),
        }
    }
}

pub struct RequestQueue<const Q: usize, T, R>
where
    R: Responder<T>,
{
    queue: [Option<Request<T, R>>; Q],
}

impl<const Q: usize, T, R: Responder<T>> RequestQueue<Q, T, R> {
    pub fn new() -> Self {
        Self {
            queue: core::array::from_fn(|_| None),
        }
    }

    pub fn response(&mut self, id: PacketId, item: T) {
        let request = self
            .queue
            .iter_mut()
            .find(|slot| slot.as_ref().is_some_and(|item| item.id == id))
            .and_then(|slot| slot.take());

        if let Some(request) = request {
            request.responder.respond(id, item);
        }
    }

    pub fn request(
        &mut self,
        id: PacketId,
        current_time: NetworkTime,
        timeout: core::time::Duration,
        responder: R,
    ) -> Result<(), NetworkError> {
        self.queue.iter_mut().for_each(|slot| {
            if slot
                .as_ref()
                .is_some_and(|req| network_time_elapsed(req.start_time, current_time, req.timeout))
            {
                *slot = None;
            }
        });

        if let Some(slot) = self.queue.iter_mut().find(|x| x.is_none()) {
            *slot = Some(Request::new(id, current_time, timeout, responder));
            Ok(())
        } else {
            Err(NetworkError::Busy)
        }
    }
}
