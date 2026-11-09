// gunzip with an output cap. Apple's Compression framework only speaks raw
// DEFLATE, so the gzip framing (RFC 1952: header, optional fields, CRC-32 +
// size trailer) is handled here, and inflation is streamed so a gzip bomb
// stops at `maxBytes` instead of filling memory.
//
// One member only: the Compression decoder swallows the input after the
// deflate stream's end without saying where that end was, so the trailer is
// read from the last 8 bytes. Both producers (the Mac's flate2 GzEncoder and
// the web app's CompressionStream) write a single member; anything else
// (several members, junk after the data) fails the CRC/size check.
import Compression
import Foundation

enum Gzip {
    struct FormatError: Error {}

    /** Decompress a single-member gzip. Throws FormatError on anything
     *  malformed and SyncError(.badSnapshot) once the output passes `maxBytes`. */
    static func decompress(_ input: Data, maxBytes: Int) throws -> Data {
        let b = [UInt8](input)
        var p = 0
        func need(_ n: Int) throws { if p + n > b.count { throw FormatError() } }
        try need(10)
        guard b[0] == 0x1F, b[1] == 0x8B, b[2] == 8 else { throw FormatError() }
        let flags = b[3]
        guard flags & 0xE0 == 0 else { throw FormatError() } // reserved bits
        p = 10
        if flags & 0x04 != 0 { // FEXTRA
            try need(2)
            let xlen = Int(b[p]) | Int(b[p + 1]) << 8
            p += 2
            try need(xlen)
            p += xlen
        }
        for bit: UInt8 in [0x08, 0x10] where flags & bit != 0 { // FNAME, FCOMMENT: zero-terminated
            guard let nul = b[p...].firstIndex(of: 0) else { throw FormatError() }
            p = nul + 1
        }
        if flags & 0x02 != 0 { // FHCRC
            try need(2)
            p += 2
        }
        // Deflate data, then the 8-byte trailer.
        try need(8)
        let t = b.count - 8
        var out = Data()
        try inflate(b, from: p, to: b.count, into: &out, maxBytes: maxBytes)
        let crc = UInt32(b[t]) | UInt32(b[t + 1]) << 8 | UInt32(b[t + 2]) << 16 | UInt32(b[t + 3]) << 24
        let size = UInt32(b[t + 4]) | UInt32(b[t + 5]) << 8 | UInt32(b[t + 6]) << 16 | UInt32(b[t + 7]) << 24
        guard CRC32.checksum(out) == crc, UInt32(truncatingIfNeeded: out.count) == size else {
            throw FormatError()
        }
        return out
    }

    /** Inflate the raw DEFLATE stream in b[from..<to]; it must reach its end. */
    private static func inflate(_ b: [UInt8], from: Int, to: Int, into out: inout Data, maxBytes: Int) throws {
        let stream = UnsafeMutablePointer<compression_stream>.allocate(capacity: 1)
        defer { stream.deallocate() }
        guard compression_stream_init(stream, COMPRESSION_STREAM_DECODE, COMPRESSION_ZLIB) == COMPRESSION_STATUS_OK else {
            throw FormatError()
        }
        defer { compression_stream_destroy(stream) }
        let chunk = 64 * 1024
        let dst = UnsafeMutablePointer<UInt8>.allocate(capacity: chunk)
        defer { dst.deallocate() }
        let srcCount = to - from
        try b.withUnsafeBufferPointer { buf in
            guard let base = buf.baseAddress else { throw FormatError() }
            stream.pointee.src_ptr = base + from
            stream.pointee.src_size = srcCount
            while true {
                stream.pointee.dst_ptr = dst
                stream.pointee.dst_size = chunk
                let srcBefore = stream.pointee.src_size
                // All input is here; FINALIZE makes a cut-off stream an error
                // instead of a wait for more.
                let status = compression_stream_process(stream, Int32(COMPRESSION_STREAM_FINALIZE.rawValue))
                let n = chunk - stream.pointee.dst_size
                if n > 0 {
                    if out.count + n > maxBytes {
                        throw SyncError(
                            kind: .badSnapshot,
                            message: "The synced data unpacks to more than \(maxBytes / 1_048_576) MB."
                        )
                    }
                    out.append(dst, count: n)
                }
                switch status {
                case COMPRESSION_STATUS_END:
                    return
                case COMPRESSION_STATUS_OK:
                    // No progress at all: truncated or stuck, never loop on it.
                    if n == 0 && stream.pointee.src_size == srcBefore { throw FormatError() }
                default:
                    throw FormatError()
                }
            }
        }
    }
}

enum CRC32 {
    private static let table: [UInt32] = (0..<256).map { i -> UInt32 in
        var c = UInt32(i)
        for _ in 0..<8 { c = c & 1 != 0 ? 0xEDB8_8320 ^ (c >> 1) : c >> 1 }
        return c
    }

    static func checksum(_ data: Data) -> UInt32 {
        var c: UInt32 = 0xFFFF_FFFF
        for byte in data { c = table[Int((c ^ UInt32(byte)) & 0xFF)] ^ (c >> 8) }
        return c ^ 0xFFFF_FFFF
    }
}
