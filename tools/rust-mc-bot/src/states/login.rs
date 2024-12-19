use crate::{Bot, Compression, packet_utils::Buf};

// c2s
pub fn write_handshake_packet(
    protocol_version: u32,
    server_address: &str,
    server_port: u16,
    next_state: u32,
) -> Buf {
    let mut buf = Buf::with_length((1 + 4 + server_address.len() + 2 + 4) as u32);
    buf.write_packet_id(0x00);

    buf.write_var_u32(protocol_version);
    buf.write_sized_str(server_address);
    buf.write_u16(server_port);
    buf.write_var_u32(next_state);

    buf
}

pub fn write_login_start_packet(username: &str) -> Buf {
    let mut buf = Buf::with_length(1 + username.len() as u32);
    buf.write_packet_id(0x00);

    buf.write_sized_str(username);
    buf.write_bool(false);

    buf
}

// s2c

// 0x02
pub fn process_login_success_packet(
    buffer: &mut Buf,
    bot: &mut Bot,
    _compression: &mut Compression,
) {
    let _uuid = buffer.read_u128();
    let _name = buffer.read_sized_string();
    let _properties = buffer.read_var_u32();

    bot.state = 2;
}

// 0x03
pub fn process_set_compression_packet(
    buf: &mut Buf,
    bot: &mut Bot,
    _compression: &mut Compression,
) {
    bot.compression_threshold = buf.read_var_u32().0 as i32;
}
