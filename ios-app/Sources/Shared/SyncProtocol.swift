// Token Counter cloud sync — wire protocol, the Swift side of
// src/sync/protocol.ts (docs/SYNC.md "Protocol reference").
//
// A pairing code carries the Supabase URL, its publishable key and a 32-byte
// secret K; the channel, auth token and AES key are derived from K. Never log
// a code, K or anything derived from it.
import CryptoKit
import Foundation

enum SyncErrorKind: String, Sendable {
    case badCode // not a parsable tc1 pairing code
    case forbidden // server refused the channel/token
    case decryptFailed // envelope malformed or GCM auth failed
    case badSnapshot // decrypted fine but not a snapshot this app understands
    case network // fetch failed, timed out, or a non-auth server error
    case empty // channel exists but nothing was uploaded yet
}

struct SyncError: Error, Sendable, Equatable {
    var kind: SyncErrorKind
    var message: String
    var status: Int? = nil
    /** Set when a bad-snapshot came from a newer Token Counter (update this app). */
    var newerSchema: Bool = false
}

struct PairingCode: Sendable, Equatable {
    var url: String // Supabase project URL, no trailing slash
    var apiKey: String // publishable key
    var secret: Data // K, 32 bytes
}

struct SyncKeys: Sendable, Equatable {
    var url: String
    var apiKey: String
    var channel: String // 32 lowercase hex chars
    var token: String // base64url, no padding
    var encKey: Data // AES-256 key bytes
}

enum SyncProtocol {
    static let codePrefix = "tc1."
    static let envelopeVersion: UInt8 = 1
    static let nonceLength = 12
    static let tagLength = 16
    /** The server refuses larger payloads, so anything bigger is not ours. */
    static let maxPayloadChars = 2_000_000
    /** Unpacked snapshot limit: real ones are tens of KB; a gzip bomb stops here. */
    static let maxSnapshotBytes = 32 * 1024 * 1024

    // MARK: pairing code + key derivation

    /** Parse a `tc1.` pairing code. All whitespace is ignored, unknown JSON
     *  fields too. Throws SyncError(.badCode). */
    static func parseCode(_ input: String) throws -> PairingCode {
        func bad(_ why: String) -> SyncError {
            SyncError(kind: .badCode, message: "Not a Token Counter pairing code (\(why)).")
        }
        let s = String(input.unicodeScalars.filter { !isJSWhitespace($0) }.map(Character.init))
        guard s.hasPrefix(codePrefix) else { throw bad("it should start with “tc1.”") }
        guard let payload = Base64URL.decode(String(s.dropFirst(codePrefix.count))),
              let text = String(data: payload, encoding: .utf8),
              let any = try? JSONSerialization.jsonObject(with: Data(text.utf8), options: [.fragmentsAllowed])
        else { throw bad("it is cut off or garbled") }
        guard let o = any as? [String: Any] else { throw bad("no payload") }
        guard let v = JSONValue.number(o["v"]), v == 1 else {
            throw bad("unsupported version \(JSONValue.describe(o["v"]))")
        }
        guard let u = o["u"] as? String, !u.isEmpty else { throw bad("missing server URL") }
        guard let k = o["k"] as? String, !k.isEmpty else { throw bad("missing server key") }
        guard let sec = o["s"] as? String, !sec.isEmpty else { throw bad("missing secret") }
        guard let url = URL(string: u), let scheme = url.scheme?.lowercased(), url.host() != nil else {
            throw bad("invalid server URL")
        }
        guard scheme == "https" || scheme == "http" else { throw bad("invalid server URL") }
        guard let secret = Base64URL.decode(sec) else { throw bad("invalid secret") }
        guard secret.count == 32 else { throw bad("secret is not 32 bytes") }
        var trimmed = u
        while trimmed.hasSuffix("/") { trimmed.removeLast() }
        return PairingCode(url: trimmed, apiKey: k, secret: secret)
    }

    /** A pasted or scanned code: whitespace dropped, and the code taken out of
     *  a whole `…#pair=tc1.…` link (the Mac's QR code is such a link). */
    static func extractCode(_ raw: String) -> String {
        let s = String(raw.unicodeScalars.filter { !isJSWhitespace($0) }.map(Character.init))
        guard let m = s.firstMatch(of: /[#&?]pair=([^&#]+)/) else { return s }
        let v = String(m.1)
        return v.removingPercentEncoding ?? v
    }

    static func deriveKeys(_ code: PairingCode) -> SyncKeys {
        func labelled(_ label: String) -> Data {
            Data(SHA256.hash(data: Data(label.utf8) + code.secret))
        }
        let ch = labelled("tc-sync/channel/v1")
        return SyncKeys(
            url: code.url,
            apiKey: code.apiKey,
            channel: ch.prefix(16).map { String(format: "%02x", $0) }.joined(),
            token: Base64URL.encode(labelled("tc-sync/auth/v1")),
            encKey: labelled("tc-sync/enc/v1")
        )
    }

    static func deriveKeys(_ code: String) throws -> SyncKeys {
        deriveKeys(try parseCode(code))
    }

    // MARK: envelope

    static func aad(_ keys: SyncKeys) -> Data {
        Data("tc-sync/snapshot/v1:\(keys.channel)".utf8)
    }

    /** Encrypt raw bytes into an envelope string. `nonce` is for test vectors only. */
    static func sealBytes(_ plain: Data, keys: SyncKeys, nonce: Data? = nil) throws -> String {
        let n = try nonce.map { try AES.GCM.Nonce(data: $0) } ?? AES.GCM.Nonce()
        let box = try AES.GCM.seal(plain, using: SymmetricKey(data: keys.encKey), nonce: n, authenticating: aad(keys))
        guard let combined = box.combined else { throw SyncError(kind: .decryptFailed, message: "Could not seal.") }
        return (Data([envelopeVersion]) + combined).base64EncodedString()
    }

    /** Decrypt an envelope string to raw bytes (still gzip'd for snapshots). */
    static func openBytes(_ payload: String, keys: SyncKeys) throws -> Data {
        func fail(_ why: String) -> SyncError { SyncError(kind: .decryptFailed, message: why) }
        if payload.utf16.count > maxPayloadChars {
            throw SyncError(kind: .badSnapshot, message: "The synced data is larger than the server allows.")
        }
        guard let raw = decodeBase64(payload.trimmingCharacters(in: .whitespacesAndNewlines)) else {
            throw fail("The synced data is not valid base64.")
        }
        guard raw.count >= 1 + nonceLength + tagLength else { throw fail("The synced data is truncated.") }
        guard raw[raw.startIndex] == envelopeVersion else {
            throw fail("Unsupported envelope version \(raw[raw.startIndex]).")
        }
        do {
            let box = try AES.GCM.SealedBox(combined: raw.dropFirst())
            return try AES.GCM.open(box, using: SymmetricKey(data: keys.encKey), authenticating: aad(keys))
        } catch {
            throw fail("The pairing code doesn't match the data on the server.")
        }
    }

    /** Envelope → validated snapshot. Throws SyncError(.decryptFailed | .badSnapshot). */
    static func openEnvelope(_ payload: String, keys: SyncKeys) throws -> Snapshot {
        let plain = try openBytes(payload, keys: keys)
        let json: Data
        do {
            json = try Gzip.decompress(plain, maxBytes: maxSnapshotBytes)
        } catch let e as SyncError {
            throw e
        } catch {
            throw SyncError(kind: .badSnapshot, message: "The synced data could not be read (not gzip’d JSON).")
        }
        return try Snapshot.parse(json)
    }

    // MARK: helpers

    /** Standard base64 like atob: padding optional, nothing else tolerated. */
    static func decodeBase64(_ s: String) -> Data? {
        guard s.wholeMatch(of: /[A-Za-z0-9+\/]*={0,2}/) != nil else { return nil }
        var t = s
        while t.hasSuffix("=") { t.removeLast() }
        if t.count % 4 == 1 { return nil }
        t += String(repeating: "=", count: (4 - t.count % 4) % 4)
        return Data(base64Encoded: t)
    }

    /** JavaScript's `\s` (what the web app strips from codes). */
    static func isJSWhitespace(_ u: Unicode.Scalar) -> Bool {
        switch u.value {
        case 0x09...0x0D, 0x20, 0xA0, 0x1680, 0x2000...0x200A, 0x2028, 0x2029, 0x202F, 0x205F, 0x3000, 0xFEFF:
            return true
        default:
            return false
        }
    }
}

enum Base64URL {
    static func encode(_ d: Data) -> String {
        d.base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }

    /** base64url with or without padding; nil on anything else. */
    static func decode(_ s: String) -> Data? {
        guard s.wholeMatch(of: /[A-Za-z0-9_\-]*={0,2}/) != nil else { return nil }
        var t = s
        while t.hasSuffix("=") { t.removeLast() }
        if t.count % 4 == 1 { return nil }
        t = t.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        t += String(repeating: "=", count: (4 - t.count % 4) % 4)
        return Data(base64Encoded: t)
    }
}

/** JSON values from JSONSerialization, read the way JavaScript's typeof does
 *  (an NSNumber holding a Bool is not a number). */
enum JSONValue {
    static func isBool(_ v: Any?) -> Bool {
        guard let n = v as? NSNumber else { return false }
        return CFGetTypeID(n) == CFBooleanGetTypeID()
    }

    /** A finite number, else nil. */
    static func number(_ v: Any?) -> Double? {
        guard let n = v as? NSNumber, !isBool(v) else { return nil }
        let d = n.doubleValue
        return d.isFinite ? d : nil
    }

    static func string(_ v: Any?) -> String? { v as? String }

    static func object(_ v: Any?) -> [String: Any]? { v as? [String: Any] }

    static func describe(_ v: Any?) -> String {
        if v == nil || v is NSNull { return v == nil ? "undefined" : "null" }
        if isBool(v) { return (v as? Bool) == true ? "true" : "false" }
        if let n = number(v) { return n == n.rounded() && abs(n) < 1e15 ? String(Int64(n)) : String(n) }
        if let s = v as? String { return s }
        return "[object Object]"
    }
}
