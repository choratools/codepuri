pub fn minimal_dex() -> Vec<u8> {
    minimal_dex_version(35)
}

pub fn minimal_dex_v41() -> Vec<u8> {
    minimal_dex_version(41)
}

fn minimal_dex_version(version: u32) -> Vec<u8> {
    let header_size = if version == 41 { 120 } else { 112 };
    let string_ids_off = header_size;
    let type_ids_off = string_ids_off + 4;
    let class_defs_off = type_ids_off + 4;
    let map_off = class_defs_off + 32;
    let string_data_off = map_off + 76;
    let file_size = string_data_off + 5;

    let mut bytes = vec![0_u8; file_size];
    bytes[0..4].copy_from_slice(b"dex\n");
    bytes[4..7].copy_from_slice(format!("{version:03}").as_bytes());
    put_u32(&mut bytes, 32, u32::try_from(file_size).unwrap_or(0));
    put_u32(&mut bytes, 36, u32::try_from(header_size).unwrap_or(0));
    put_u32(&mut bytes, 40, 0x1234_5678);
    put_u32(&mut bytes, 52, u32::try_from(map_off).unwrap_or(0));
    put_u32(&mut bytes, 56, 1);
    put_u32(&mut bytes, 60, u32::try_from(string_ids_off).unwrap_or(0));
    put_u32(&mut bytes, 64, 1);
    put_u32(&mut bytes, 68, u32::try_from(type_ids_off).unwrap_or(0));
    put_u32(&mut bytes, 96, 1);
    put_u32(&mut bytes, 100, u32::try_from(class_defs_off).unwrap_or(0));
    if version == 41 {
        put_u32(&mut bytes, 112, u32::try_from(file_size).unwrap_or(0));
    } else {
        put_u32(
            &mut bytes,
            104,
            u32::try_from(file_size - map_off).unwrap_or(0),
        );
        put_u32(&mut bytes, 108, u32::try_from(map_off).unwrap_or(0));
    }

    put_u32(
        &mut bytes,
        string_ids_off,
        u32::try_from(string_data_off).unwrap_or(0),
    );
    put_u32(&mut bytes, type_ids_off, 0);
    put_u32(&mut bytes, class_defs_off, 0);
    put_u32(&mut bytes, class_defs_off + 4, 1);
    put_u32(&mut bytes, class_defs_off + 8, u32::MAX);
    put_u32(&mut bytes, class_defs_off + 16, u32::MAX);

    put_u32(&mut bytes, map_off, 6);
    put_map(&mut bytes, map_off + 4, 0x0000, 1, 0);
    put_map(
        &mut bytes,
        map_off + 16,
        0x0001,
        1,
        u32::try_from(string_ids_off).unwrap_or(0),
    );
    put_map(
        &mut bytes,
        map_off + 28,
        0x0002,
        1,
        u32::try_from(type_ids_off).unwrap_or(0),
    );
    put_map(
        &mut bytes,
        map_off + 40,
        0x0006,
        1,
        u32::try_from(class_defs_off).unwrap_or(0),
    );
    put_map(
        &mut bytes,
        map_off + 52,
        0x1000,
        1,
        u32::try_from(map_off).unwrap_or(0),
    );
    put_map(
        &mut bytes,
        map_off + 64,
        0x2002,
        1,
        u32::try_from(string_data_off).unwrap_or(0),
    );
    bytes[string_data_off..].copy_from_slice(&[3, b'L', b'x', b';', 0]);
    bytes
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_map(bytes: &mut [u8], offset: usize, item_type: u16, size: u32, item_offset: u32) {
    bytes[offset..offset + 2].copy_from_slice(&item_type.to_le_bytes());
    put_u32(bytes, offset + 4, size);
    put_u32(bytes, offset + 8, item_offset);
}
