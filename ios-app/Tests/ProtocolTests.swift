// Interop with the web app and the Mac: every vector here was computed by the
// TypeScript in src/ (scripts/make-test-vectors.mjs) or by the Rust side
// (src-tauri/tests/fixtures/sync_envelope_vector.json).
import Foundation
import Testing
@testable import Tokens

enum Fixtures {
    static func json(_ name: String) throws -> [String: Any] {
        let bundle = Bundle(for: BundleToken.self)
        let url = try #require(bundle.url(forResource: name, withExtension: "json"))
        return try #require(try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any])
    }

    static var vectors: [String: Any] { get throws { try json("vectors") } }
}

private final class BundleToken {}

/** JSON-equal (NSDictionary/NSArray/NSNumber isEqual semantics). */
func jsonEqual(_ a: Any?, _ b: Any?) -> Bool {
    switch (a, b) {
    case (nil, nil): return true
    case let (x as NSObject, y as NSObject): return x.isEqual(y)
    default: return false
    }
}

@Suite struct ProtocolTests {
    @Test func derivedKeysMatchTheWebApp() throws {
        let k = try #require(try Fixtures.vectors["keys"] as? [String: String])
        let keys = try SyncProtocol.deriveKeys(try #require(k["code"]))
        #expect(keys.channel == k["channel"])
        #expect(keys.token == k["token"])
        #expect(keys.encKey.map { String(format: "%02x", $0) }.joined() == k["encKeyHex"])
        #expect(keys.url == "https://example.supabase.co")
        #expect(keys.apiKey == "sb_publishable_test")
    }

    @Test func parseCodeAgreesWithTheWebApp() throws {
        let cases = try #require(try Fixtures.vectors["parseCode"] as? [[String: Any]])
        #expect(cases.count > 15)
        for c in cases {
            let input = try #require(c["input"] as? String)
            if c["ok"] as? Bool == true {
                let p = try SyncProtocol.parseCode(input)
                #expect(p.url == c["url"] as? String, "\(input)")
                #expect(p.apiKey == c["apiKey"] as? String)
                #expect(p.secret.map { String(format: "%02x", $0) }.joined() == c["secretHex"] as? String)
            } else {
                let message = try #require(c["message"] as? String)
                #expect(throws: SyncError(kind: .badCode, message: message)) {
                    try SyncProtocol.parseCode(input)
                }
            }
        }
    }

    @Test func extractCodeAgreesWithTheWebApp() throws {
        let cases = try #require(try Fixtures.vectors["extractCode"] as? [[String: String]])
        for c in cases {
            #expect(SyncProtocol.extractCode(c["input"]!) == c["output"]!, "\(c["input"]!)")
        }
    }

    @Test func opensTheWebAppsEnvelope() throws {
        let v = try Fixtures.vectors
        let keys = try SyncProtocol.deriveKeys(try #require((v["keys"] as? [String: String])?["code"]))
        let env = try #require(v["envelope"] as? [String: Any])
        let snap = try SyncProtocol.openEnvelope(try #require(env["payload"] as? String), keys: keys)
        let decoded = try JSONSerialization.jsonObject(with: snap.source)
        #expect(jsonEqual(decoded, env["json"]))
        #expect(snap.deviceName == "Mac Studio")
        #expect(snap.usage.count == 5)
    }

    @Test func opensTheRustFixture() throws {
        let fx = try Fixtures.json("sync_envelope_vector")
        let keys = try SyncProtocol.deriveKeys(try #require(fx["code"] as? String))
        let snap = try SyncProtocol.openEnvelope(try #require(fx["payload"] as? String), keys: keys)
        #expect(jsonEqual(try JSONSerialization.jsonObject(with: snap.source), fx["json"]))
        #expect(snap.claudeQuota?.weekly?.usedPct == 44)
        #expect(snap.usage[.today]?.totalTokens == 27265.8)
    }

    @Test func helloVector() throws {
        let v = try Fixtures.vectors
        let keys = try SyncProtocol.deriveKeys(try #require((v["keys"] as? [String: String])?["code"]))
        let plain = try SyncProtocol.openBytes(try #require(v["helloEnvelope"] as? String), keys: keys)
        #expect(String(data: plain, encoding: .utf8) == "hello")
        let nonce = Data((0..<12).map { UInt8($0) })
        #expect(try SyncProtocol.sealBytes(Data("hello".utf8), keys: keys, nonce: nonce) == v["helloEnvelope"] as? String)
    }

    @Test func refusesWhatItShould() throws {
        let v = try Fixtures.vectors
        let keys = try SyncProtocol.deriveKeys(try #require((v["keys"] as? [String: String])?["code"]))
        func kind(_ payload: String) -> SyncError? {
            do {
                _ = try SyncProtocol.openEnvelope(payload, keys: keys)
                return nil
            } catch {
                return error as? SyncError
            }
        }
        #expect(kind(try #require(v["wrongKeyPayload"] as? String))?.kind == .decryptFailed)
        let newer = kind(try #require(v["newerSchemaPayload"] as? String))
        #expect(newer?.kind == .badSnapshot && newer?.newerSchema == true)
        #expect(newer?.message == "The main Mac runs a newer Token Counter (data format 2).")
        #expect(kind(try #require(v["notGzipPayload"] as? String))?.kind == .badSnapshot)

        let good = try #require((v["envelope"] as? [String: Any])?["payload"] as? String)
        var raw = try #require(Data(base64Encoded: good))
        raw[raw.count - 1] ^= 1
        #expect(kind(raw.base64EncodedString())?.message == "The pairing code doesn't match the data on the server.")
        raw = try #require(Data(base64Encoded: good))
        raw[0] = 2
        #expect(kind(raw.base64EncodedString())?.message == "Unsupported envelope version 2.")
        #expect(kind(Data(raw.prefix(20)).base64EncodedString())?.message == "The synced data is truncated.")
        #expect(kind("not base64!")?.message == "The synced data is not valid base64.")
        #expect(kind(String(repeating: "A", count: SyncProtocol.maxPayloadChars + 4))?.kind == .badSnapshot)
        // Padding is optional, like atob.
        #expect(kind(String(good.prefix { $0 != "=" })) == nil)
    }

    @Test func sealOpenRoundTrip() throws {
        let keys = SyncProtocol.deriveKeys(PairingCode(url: "https://x.supabase.co", apiKey: "k", secret: Data(repeating: 9, count: 32)))
        let a = try SyncProtocol.sealBytes(Data("abc".utf8), keys: keys)
        let b = try SyncProtocol.sealBytes(Data("abc".utf8), keys: keys)
        #expect(a != b) // fresh nonce each time
        #expect(try SyncProtocol.openBytes(a, keys: keys) == Data("abc".utf8))
    }
}

@Suite struct GzipTests {
    private func vec(_ k: String) throws -> Data {
        let g = try #require(try Fixtures.vectors["gzip"] as? [String: Any])
        let b64 = try #require(g[k] as? String)
        return try #require(Data(base64Encoded: b64))
    }

    @Test func framingVariants() throws {
        let expected = try vec("expectedB64")
        for k in ["plain", "allFields"] {
            #expect(try Gzip.decompress(try vec(k), maxBytes: 1 << 25) == expected, "\(k)")
        }
    }

    @Test func malformedInputIsRejected() throws {
        // Neither producer writes several members (see Gzip.swift).
        for k in ["badCrc", "truncated", "junkAfter", "twoMembers"] {
            #expect(throws: Gzip.FormatError.self, "\(k)") { try Gzip.decompress(try vec(k), maxBytes: 1 << 25) }
        }
        #expect(throws: Gzip.FormatError.self) { try Gzip.decompress(Data(), maxBytes: 10) }
        #expect(throws: Gzip.FormatError.self) { try Gzip.decompress(Data("{}".utf8), maxBytes: 10) }
    }

    @Test func bombStopsAtTheCap() throws {
        #expect(throws: SyncError(kind: .badSnapshot, message: "The synced data unpacks to more than 32 MB.")) {
            try Gzip.decompress(try vec("bomb"), maxBytes: SyncProtocol.maxSnapshotBytes)
        }
    }
}

@Suite struct SnapshotTests {
    @Test func coercesLikeTheWebApp() throws {
        let cases = try #require(try Fixtures.vectors["sanitize"] as? [[String: Any]])
        let first = try #require(cases.first)
        let input = try #require(first["input"])
        let ts = try #require(first["output"] as? [String: Any])
        let s = try Snapshot.validate(input, source: Data())
        #expect(s.deviceName == (ts["device"] as? [String: Any])?["name"] as? String)
        let tsToday = try #require((ts["usage"] as? [String: Any])?["today"] as? [String: Any])
        let today = try #require(s.usage[.today])
        #expect(s.usage[.week] == nil && (ts["usage"] as? [String: Any])?["week"] == nil)
        let tsModels = try #require(tsToday["byModel"] as? [String: [String: Any]])
        #expect(today.byModel.map(\.id) == tsModels.keys.sorted())
        let a = try #require(today.byModel.first?.usage)
        #expect(a.inK == tsModels["a"]?["in"] as? Double)
        #expect(a.outK == tsModels["a"]?["out"] as? Double)
        #expect(a.cacheWriteK == tsModels["a"]?["cacheWrite"] as? Double)
        #expect(a.cacheReadK == tsModels["a"]?["cacheRead"] as? Double)
        #expect(a.cost == nil && tsModels["a"]?["cost"] == nil)
        let tsTools = try #require(tsToday["byTool"] as? [[String: Any]])
        #expect(today.byTool.map(\.name) == tsTools.map { $0["name"] as? String ?? "" })
        #expect(today.byTool.map(\.calls) == tsTools.map { $0["calls"] as? Double ?? -1 })
        #expect(today.sparkline == tsToday["sparkline"] as? [Double])
        #expect(today.sparklineCodex == nil && tsToday["sparklineCodex"] == nil)
        #expect(today.totalCost == tsToday["totalCost"] as? Double)
        #expect(today.deltaPct == nil && tsToday["deltaPct"] is NSNull)
        #expect(today.rangeLabel == tsToday["rangeLabel"] as? String)
        let tsQuota = try #require(ts["claudeQuota"] as? [String: Any])
        #expect(s.claudeQuota?.status == tsQuota["status"] as? String)
        #expect(s.claudeQuota?.fiveHour == nil && tsQuota["fiveHour"] is NSNull)
        #expect(s.claudeQuota?.weekly?.usedPct == (tsQuota["weekly"] as? [String: Any])?["usedPct"] as? Double)

        let second = try #require(cases.last)
        let s2 = try Snapshot.validate(try #require(second["input"]), source: Data())
        let ts2 = try #require(second["output"] as? [String: Any])
        #expect(s2.appVersion == nil && (ts2["device"] as? [String: Any])?["appVersion"] == nil)
        #expect(s2.plan == nil && ts2["plan"] == nil)
        #expect(s2.usage.isEmpty)
    }

    @Test func rejectsLikeTheWebApp() throws {
        let cases = try #require(try Fixtures.vectors["sanitizeRejects"] as? [[String: Any]])
        for c in cases {
            #expect(c["ok"] as? Bool == false)
            #expect(throws: SyncError.self) { try Snapshot.validate(c["input"] ?? NSNull(), source: Data()) }
        }
    }

    @Test func demoSnapshotMatchesTheWebAppsReading() throws {
        let env = try #require(try Fixtures.vectors["envelope"] as? [String: Any])
        let ts = try #require(env["sanitized"] as? [String: Any])
        let data = try JSONSerialization.data(withJSONObject: try #require(env["json"]))
        let s = try Snapshot.parse(data)
        let tsUsage = try #require(ts["usage"] as? [String: [String: Any]])
        for r in UsageRange.allCases {
            let u = try #require(s.usage[r])
            let t = try #require(tsUsage[r.rawValue])
            #expect(u.totalCost == t["totalCost"] as? Double, "\(r)")
            #expect(u.totalTokens == t["totalTokens"] as? Double, "\(r)")
            #expect(u.sparkline == t["sparkline"] as? [Double], "\(r)")
            #expect(u.rangeLabel == t["rangeLabel"] as? String, "\(r)")
            #expect(u.byModel.count == (t["byModel"] as? [String: Any])?.count, "\(r)")
        }
    }
}

@Suite struct DateTests {
    @Test func parsesTheShapesTheMacWrites() throws {
        let base = try #require(ISODate.parse("2026-11-09T11:50:00Z"))
        #expect(base.timeIntervalSince1970 == 1_794_225_000)
        #expect(abs(ISODate.parse("2026-11-09T11:50:00.612317+00:00")!.timeIntervalSince(base) - 0.612317) < 1e-6)
        #expect(abs(ISODate.parse("2026-11-09T06:50:00.906-05:00")!.timeIntervalSince(base) - 0.906) < 1e-6)
        #expect(ISODate.parse("2026-11-09T17:20:00+05:30") == base)
        #expect(ISODate.parse("2026-11-09T11:50:00.5Z")!.timeIntervalSince1970 == 1_794_225_000.5)
        #expect(ISODate.parse("2026-11-09T11:50:00+0000") == base)
        #expect(ISODate.parse("2026-02-30T00:00:00Z") == nil)
        #expect(ISODate.parse("nope") == nil)
        #expect(ISODate.parse(nil) == nil)
    }
}

@Suite struct CacheTests {
    private func tempURL() -> URL {
        FileManager.default.temporaryDirectory.appendingPathComponent("tc-cache-test-\(UUID().uuidString).json")
    }

    @Test func roundTripKeepsTheExactBytes() throws {
        let url = tempURL()
        defer { try? FileManager.default.removeItem(at: url) }
        // -1e999 parses to -infinity; writing that back as JSON would raise.
        let json = Data(#"{"schema":1,"generatedAt":"2026-11-09T08:00:00Z","device":{"name":"M"},"usage":{"today":{"totalCost":-1e999,"other":-1e999,"totalTokens":5}}}"#.utf8)
        let snap = try Snapshot.parse(json)
        #expect(snap.usage[.today]?.totalCost == 0)
        #expect(snap.usage[.today]?.totalTokens == 5)
        let fetched = Date(timeIntervalSince1970: 1_794_225_000)
        SnapshotCache.write(CachedSnapshot(channel: "abc", version: 3, fetchedAt: fetched, snapshot: snap), url: url)
        let back = try #require(SnapshotCache.read(url: url))
        #expect(back.channel == "abc")
        #expect(back.version == 3)
        #expect(back.fetchedAt == fetched)
        #expect(back.snapshot.source == json)
    }

    @Test func aBrokenCacheIsRemovedNotTrusted() throws {
        for content in ["{not json", #"{"channel":"c","version":1e30,"fetchedAt":1,"snapshot":"e30="}"#, #"{"channel":"c","version":1,"fetchedAt":1,"snapshot":"e30="}"#] {
            let url = tempURL()
            try Data(content.utf8).write(to: url)
            #expect(SnapshotCache.read(url: url) == nil, "\(content)")
            #expect(!FileManager.default.fileExists(atPath: url.path), "\(content)")
        }
        #expect(SnapshotCache.read(url: tempURL()) == nil)
    }

    @Test func aHugeSchemaIsANewerFormatNotACrash() throws {
        for (text, shown) in [("1e19", "10000000000000000000"), ("9223372036854775807", "9223372036854775808"), ("2.9", "2")] {
            let json = Data(#"{"schema":\#(text),"generatedAt":"x","usage":{}}"#.utf8)
            #expect(throws: SyncError(kind: .badSnapshot, message: "The main Mac runs a newer Token Counter (data format \(shown)).", newerSchema: true)) {
                try Snapshot.parse(json)
            }
        }
    }
}
