use crate::{Bot, Compression, packet_utils::Buf};

pub fn process_status_response(buffer: &mut Buf, _bot: &mut Bot, _compression: &mut Compression) {
    let server_response = buffer.read_sized_string();
    tracing::info!("got response {server_response}");
}

pub fn process_pong(buffer: &mut Buf, _bot: &mut Bot, _compression: &mut Compression) {
    let payload = buffer.read_sized_string();
    tracing::info!("got pong {payload}");
}
