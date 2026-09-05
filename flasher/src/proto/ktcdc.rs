//! The **byte-exact** KTMicro CDC bootloader download framing (roadmap M1 → M1.5).
//!
//! Recovered by decompiling the vendor `JadeAudio JA11 Upgrade Tool.exe`
//! (`FUN_0054d7d0` data packet, `FUN_005733a0` final packet, `FUN_00b8c380` state
//! machine) — see `docs/CDC-PROTOCOL.md` and `re_findings.md`. Every constant here is
//! read straight from the vendor binary; the packet math is unit-tested below and against
//! the golden `bootdiag --replay` fixtures before it is ever allowed near hardware.
//!
//! This module is pure (no I/O). `main.rs` wires it to the OrbStack/Linux serial pipe.

/// Fixed command tokens (lead byte + 3 ASCII), read from the vendor `.rdata`.
pub mod token {
    pub const KTM: [u8; 4] = [0x1e, 0x4b, 0x54, 0x4d]; // handshake / hello
    pub const VER: [u8; 4] = [0xf0, 0x56, 0x45, 0x52]; // get version   (flag=1 path)
    pub const CHP: [u8; 4] = [0xd2, 0x43, 0x48, 0x50]; // chip id
    pub const KEY: [u8; 4] = [0xf0, 0x4b, 0x45, 0x59]; // key / auth
    pub const PWO: [u8; 4] = [0x3c, 0x50, 0x57, 0x4f]; // power / prepare
    pub const KSTA: [u8; 4] = [0x4b, 0x53, 0x54, 0x41]; // start programming
    pub const STP: [u8; 4] = [0x96, 0x53, 0x54, 0x50]; // stop programming
    pub const INF: [u8; 4] = [0xf0, 0x49, 0x4e, 0x46]; // info / verify (flag=1 path)
    pub const RESET: [u8; 4] = [0x5a, 0x52, 0x53, 0x54]; // "ZRST" — reboot to firmware
    /// 10-byte erase/region setup blob (state 4).
    pub const ERASE_SETUP: [u8; 10] = [0x2d, 0x29, 0x00, 0x10, 0x0e, 0x15, 0x00, 0x60, 0x00, 0xbc];
}

/// Response/ACK bytes the vendor checks for via `QByteArray::indexOf`.
pub mod ack {
    pub const ACCEPT: u8 = 0x78; // command accepted / ready
    pub const BLOCK: u8 = 0xa5; // per-data-block ACK
    pub const DONE: u8 = 0x03; // status / done
}

/// Payload chunk size for a full data block.
pub const BLOCK: usize = 0x400; // 1024
/// The 16-byte image header is skipped by the data loop and written LAST by the final packet.
pub const HEADER_SKIP: usize = 0x10;
/// First data block payload length: `BLOCK - HEADER_SKIP` (image bytes 0x10..0x400).
pub const FIRST_BLOCK_LEN: usize = BLOCK - HEADER_SKIP; // 0x3F0 = 1008
/// A "region" is 32 blocks (32 KB); block indices that are a multiple of this are region
/// boundaries and carry the `bank` id in the header's top-3 bits instead of the 0b111 marker.
pub const BLOCKS_PER_REGION: usize = 0x20;
/// The continuation / final top-3 marker (`0b111`) — becomes `0xE0` in header byte 2.
pub const TOP3_CONT: u8 = 0b111;

/// Reflected CRC-32 table for poly 0xEDB88320 (matches vendor `DAT_0120e020`), built at
/// compile time.
const CRC32_TABLE: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
};

/// The KTMicro CRC-32 variant: reflected poly 0xEDB88320, **init = 0**, **no final XOR**
/// (differs from the standard zip/PNG CRC-32 which uses init/xorout 0xFFFFFFFF). Computed
/// over the 6-byte header **plus** payload (the header is prepended before the CRC loop).
pub fn crc32_kt(data: &[u8]) -> u32 {
    let mut crc: u32 = 0;
    for &b in data {
        crc = (crc >> 8) ^ CRC32_TABLE[((crc as u8) ^ b) as usize];
    }
    crc
}

/// Build a data packet: `[0x69, H1, H2, addr_lo, addr_mid, addr_hi] + payload + crc32_le`.
///
/// - `payload`  ≤ 1024 bytes (its length is the 13-bit `L` field).
/// - `addr`     24-bit flash address for this payload.
/// - `top3`     header byte-2 top-3 bits: the bank id on a region-boundary packet, else
///   [`TOP3_CONT`] (0b111).
pub fn build_data_packet(payload: &[u8], addr: u32, top3: u8) -> Vec<u8> {
    assert!(payload.len() <= BLOCK, "payload {} > {BLOCK}", payload.len());
    assert!(top3 <= 0b111, "top3 must be 3 bits");
    let l = payload.len() as u32;
    let mut pkt = Vec::with_capacity(6 + payload.len() + 4);
    pkt.push(0x69); // H[0]
    pkt.push((l & 0xFF) as u8); // H[1]
    pkt.push((((l >> 8) & 0x1F) as u8) | (top3 << 5)); // H[2]
    pkt.push((addr & 0xFF) as u8); // H[3]
    pkt.push(((addr >> 8) & 0xFF) as u8); // H[4]
    pkt.push(((addr >> 16) & 0xFF) as u8); // H[5]
    pkt.extend_from_slice(payload);
    let crc = crc32_kt(&pkt); // over header + payload
    pkt.extend_from_slice(&crc.to_le_bytes()); // little-endian
    pkt
}

/// The FINAL packet (`FUN_005733a0`): writes the image's first 16 bytes to `base`, last.
/// Fixed header `69 10 E0 <addr 24-bit LE>` (L=0x10, top3=0b111).
pub fn build_final_packet(image_header16: &[u8], base_addr: u32) -> Vec<u8> {
    assert_eq!(image_header16.len(), HEADER_SKIP, "final payload must be 16 bytes");
    build_data_packet(image_header16, base_addr, TOP3_CONT)
}

/// One planned outbound packet plus the metadata the driver logs / verifies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    pub bytes: Vec<u8>,
    pub addr: u32,
    pub payload_len: usize,
    pub is_final: bool,
}

/// Produce the exact ordered packet stream for `image` at write base `base` (`= flag<<15`).
///
/// Order matches the vendor loop: data block 0 (image[0x10..0x400] → base+0x10), then
/// blocks 1..N (image[k*0x400..] → base+k*0x400), then the FINAL packet (image[0..0x10]
/// → base). Concatenating the payloads in address order reconstructs `image` exactly.
pub fn plan_stream(image: &[u8], base: u32) -> Vec<Packet> {
    assert!(image.len() > HEADER_SKIP, "image too small");
    let mut out = Vec::new();
    // block 0: image[0x10 .. 0x400]  (or to end if the image is < 0x400)
    let first_end = image.len().min(BLOCK);
    let first = &image[HEADER_SKIP..first_end];
    out.push(Packet {
        bytes: build_data_packet(first, base + HEADER_SKIP as u32, bank_top3(0)),
        addr: base + HEADER_SKIP as u32,
        payload_len: first.len(),
        is_final: false,
    });
    // blocks 1..N
    let mut k = 1usize;
    while k * BLOCK < image.len() {
        let start = k * BLOCK;
        let end = image.len().min(start + BLOCK);
        let payload = &image[start..end];
        let addr = base + start as u32;
        out.push(Packet {
            bytes: build_data_packet(payload, addr, bank_top3(k)),
            addr,
            payload_len: payload.len(),
            is_final: false,
        });
        k += 1;
    }
    // FINAL: image[0..0x10] → base
    out.push(Packet {
        bytes: build_final_packet(&image[..HEADER_SKIP], base),
        addr: base,
        payload_len: HEADER_SKIP,
        is_final: true,
    });
    out
}

/// top-3 header bits for data block `k`: the bank id (1) on a region boundary, else 0b111.
/// (At the JA11 call sites `param_2 == 1`, so boundary blocks carry bank id 1 → header
/// high nibble `0x20`; all other blocks carry `0b111` → `0xE0`.)
fn bank_top3(k: usize) -> u8 {
    if k.is_multiple_of(BLOCKS_PER_REGION) {
        1
    } else {
        TOP3_CONT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Independent bitwise CRC (init=0, xorout=0) to cross-check the table-driven one.
    fn crc32_kt_bitwise(data: &[u8]) -> u32 {
        let mut crc: u32 = 0;
        for &b in data {
            crc ^= b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
        crc
    }

    #[test]
    fn crc_table_matches_bitwise() {
        for v in [
            &b""[..],
            &b"123456789"[..],
            &b"KT_Helios_v1b"[..],
            &[0x69, 0xf0, 0x23, 0x10, 0x00, 0x00][..],
        ] {
            assert_eq!(crc32_kt(v), crc32_kt_bitwise(v), "mismatch on {v:?}");
        }
        // table[1] must equal the canonical reflected value (proven byte-exact in RE).
        assert_eq!(CRC32_TABLE[1], 0x7707_3096);
    }

    #[test]
    fn header_bytes_match_decompiled_formula() {
        // A normal full block at a non-boundary index: L=0x400, top3=0b111, addr=0x8410.
        let payload = vec![0xAAu8; 0x400];
        let pkt = build_data_packet(&payload, 0x8410, TOP3_CONT);
        assert_eq!(pkt[0], 0x69);
        assert_eq!(pkt[1], 0x00); // L & 0xFF  (0x400 & 0xff = 0)
        assert_eq!(pkt[2], (0x04) | (0b111 << 5)); // (0x400>>8)&0x1f=4, |0xE0 => 0xE4
        assert_eq!(pkt[2], 0xE4);
        assert_eq!(pkt[3], 0x10); // addr lo
        assert_eq!(pkt[4], 0x84); // addr mid
        assert_eq!(pkt[5], 0x00); // addr hi
        assert_eq!(pkt.len(), 6 + 0x400 + 4);
        // last 4 bytes are the LE CRC over header+payload
        let crc = crc32_kt(&pkt[..6 + 0x400]);
        assert_eq!(&pkt[6 + 0x400..], &crc.to_le_bytes());
    }

    #[test]
    fn boundary_block_carries_bank_id_not_cont() {
        // block 0 is a region boundary (0 % 0x20 == 0) → top3 = 1 → high nibble 0x20.
        let payload = vec![0u8; FIRST_BLOCK_LEN]; // 0x3F0
        let pkt = build_data_packet(&payload, 0x10, bank_top3(0));
        assert_eq!(pkt[1], 0xF0); // 0x3F0 & 0xFF
        assert_eq!(pkt[2], 0x03 | (1 << 5)); // (0x3F0>>8)&0x1f=3, top3=1 => 0x23
        assert_eq!(pkt[2], 0x23);
    }

    #[test]
    fn final_packet_is_69_10_e0_plus_addr() {
        let hdr16 = [0x11u8; 16];
        let pkt = build_final_packet(&hdr16, 0x8000);
        assert_eq!(&pkt[..6], &[0x69, 0x10, 0xE0, 0x00, 0x80, 0x00]);
        assert_eq!(pkt.len(), 6 + 16 + 4);
        assert_eq!(&pkt[6..6 + 16], &hdr16);
    }

    #[test]
    fn plan_payloads_reconstruct_the_image_in_address_order() {
        // Build a recognisable image spanning several blocks + a partial tail.
        let n = 0x400 * 3 + 0x50; // 3 full blocks + 0x50
        let image: Vec<u8> = (0..n).map(|i| (i & 0xFF) as u8).collect();
        let base = 0x8000u32;
        let pkts = plan_stream(&image, base);

        // Reassemble: for each packet, place its payload at (addr - base) in a buffer.
        let mut recon = vec![0u8; image.len()];
        for p in &pkts {
            // recover the payload back out of the framed bytes
            let payload = &p.bytes[6..6 + p.payload_len];
            let off = (p.addr - base) as usize;
            recon[off..off + p.payload_len].copy_from_slice(payload);
            // verify the appended CRC of every packet
            let body = &p.bytes[..6 + p.payload_len];
            assert_eq!(&p.bytes[6 + p.payload_len..], &crc32_kt(body).to_le_bytes());
        }
        assert_eq!(recon, image, "planned packets do not reconstruct the image");

        // Exactly one final packet, and it writes the 16-byte header to base.
        assert_eq!(pkts.iter().filter(|p| p.is_final).count(), 1);
        let f = pkts.last().unwrap();
        assert!(f.is_final && f.addr == base && f.payload_len == HEADER_SKIP);
    }

    #[test]
    fn plan_block_count_is_correct_for_a_realistic_size() {
        // 67312-byte JA11 image → block 0 + blocks 1..=65 + final = 67 packets.
        let image = vec![0u8; 67312];
        let pkts = plan_stream(&image, 0);
        let data = pkts.iter().filter(|p| !p.is_final).count();
        // ceil(67312 / 1024) = 66 blocks covering [0..end] via block0(0x10..) + 1..65
        assert_eq!(data, 66);
        assert_eq!(pkts.len(), 67); // + final
    }
}
