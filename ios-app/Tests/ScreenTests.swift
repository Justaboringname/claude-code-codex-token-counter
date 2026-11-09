// The numbers and labels on screen must match the web app digit for digit.
// Expected values come from the web app's own TypeScript
// (scripts/make-test-vectors.mjs → Fixtures/vectors.json).
import Foundation
import Testing
@testable import Tokens

private func dbl(_ v: Any?) -> Double? { JSONValue.number(v) }

private func syncError(_ v: Any?) -> SyncError? {
    guard let o = v as? [String: Any], let k = o["kind"] as? String else { return nil }
    let kind: SyncErrorKind =
        switch k {
        case "bad-code": .badCode
        case "decrypt-failed": .decryptFailed
        case "bad-snapshot": .badSnapshot
        case "forbidden": .forbidden
        case "empty": .empty
        default: .network
        }
    return SyncError(kind: kind, message: o["message"] as? String ?? "", newerSchema: o["reason"] as? String == "newer-schema")
}

private func date(ms: Any?) -> Date { Date(timeIntervalSince1970: (dbl(ms) ?? 0) / 1000) }

@Suite struct FormatTests {
    @Test func toFixedRoundsLikeJavaScript() throws {
        let cases = try #require(try Fixtures.vectors["toFixed"] as? [[String: Any]])
        #expect(cases.count > 20)
        for c in cases {
            let x = try #require(dbl(c["x"]))
            let d = Int(try #require(dbl(c["d"])))
            #expect(JS.toFixed(x, d) == c["text"] as? String, "\(x) to \(d) digits")
        }
    }

    @Test func moneyTokensCredits() throws {
        let v = try Fixtures.vectors
        for c in try #require(v["fmtMoney"] as? [[String: Any]]) {
            let n = try #require(dbl(c["n"]))
            #expect(Fmt.money(n) == c["text"] as? String, "money \(n)")
        }
        for c in try #require(v["fmtMoney0"] as? [[String: Any]]) {
            let n = try #require(dbl(c["n"]))
            #expect(Fmt.money(n, decimals: 0) == c["text"] as? String, "money0 \(n)")
        }
        for c in try #require(v["fmtTokens"] as? [[String: Any]]) {
            let k = try #require(dbl(c["k"]))
            #expect(Fmt.tokens(k) == c["text"] as? String, "tokens \(k)")
        }
        for c in try #require(v["fmtCredits"] as? [[String: Any]]) {
            let n = try #require(dbl(c["n"]))
            #expect(Fmt.credits(n) == c["text"] as? String, "credits \(n)")
        }
    }

    @Test func relativeAge() throws {
        let now = Date(timeIntervalSince1970: 1_794_225_600)
        for c in try #require(try Fixtures.vectors["relativeAge"] as? [[String: Any]]) {
            let s = try #require(dbl(c["seconds"]))
            #expect(Fmt.relativeAge(now.addingTimeInterval(-s), now: now) == c["text"] as? String, "\(s) s")
        }
    }

    @Test func modelNamesAndColours() throws {
        let v = try Fixtures.vectors
        for c in try #require(v["modelMeta"] as? [[String: Any]]) {
            let id = try #require(c["id"] as? String)
            let m = Models.meta(id)
            #expect(m.name == c["name"] as? String, "\(id)")
            #expect(m.color == c["color"] as? String, "\(id)")
            #expect(m.family == c["family"] as? String, "\(id)")
            #expect(Models.isCodex(id) == c["isCodex"] as? Bool, "\(id)")
        }
        for c in try #require(v["liftForDark"] as? [[String: String]]) {
            #expect(Models.liftForDark(c["hex"]!) == c["out"]!)
        }
        for c in try #require(v["displayToolName"] as? [[String: String]]) {
            #expect(Tools.displayName(c["name"]!) == c["out"]!)
        }
        for c in try #require(v["costForModel"] as? [[String: Any]]) {
            let id = try #require(c["id"] as? String)
            let u = try #require(c["u"] as? [String: Any])
            let usage = ModelUsage(
                inK: dbl(u["in"]) ?? 0, outK: dbl(u["out"]) ?? 0, cacheWriteK: dbl(u["cacheWrite"]) ?? 0,
                cacheReadK: dbl(u["cacheRead"]) ?? 0, cost: dbl(u["cost"]), priceSource: nil
            )
            let want = try #require(dbl(c["cost"]))
            #expect(abs(Models.cost(id, usage) - want) < 1e-12, "\(id)")
        }
    }
}

@Suite struct ScreenTests {
    @Test func everyRangeShowsWhatTheWebAppShows() throws {
        let screens = try #require(try Fixtures.vectors["screens"] as? [[String: Any]])
        #expect(screens.count == 4)
        for sc in screens {
            let name = try #require(sc["name"] as? String)
            let json = try #require(sc["json"])
            let snap = try Snapshot.parse(try JSONSerialization.data(withJSONObject: json))
            let now = date(ms: sc["now"])
            let ranges = try #require(sc["ranges"] as? [String: [String: Any]])
            for r in UsageRange.allCases {
                let want = try #require(ranges[r.rawValue])
                let tag = "\(name)/\(r.rawValue)"
                let usage = snap.usage[r]
                #expect((usage != nil) == (want["present"] as? Bool), "\(tag) present")
                #expect(Dashboard.heroLabel(usage) == want["heroLabel"] as? String, "\(tag) label")
                #expect(Fmt.money(usage?.totalCost ?? 0) == want["money"] as? String, "\(tag) money")
                #expect("\(Fmt.tokens(usage?.totalTokens ?? 0)) tokens" == want["tokens"] as? String, "\(tag) tokens")

                let delta = usage?.deltaPct.map(Dashboard.delta)
                if let wd = want["delta"] as? [String: Any] {
                    #expect(delta?.up == wd["up"] as? Bool, "\(tag) delta up")
                    #expect(delta?.text == wd["text"] as? String, "\(tag) delta text")
                } else {
                    #expect(delta == nil, "\(tag) delta")
                }

                let spark = Dashboard.sparkSeries(usage)
                #expect((spark.codex != nil) == (want["codexOverlay"] as? Bool), "\(tag) codex overlay")
                #expect(Double(spark.claude.count) == dbl(want["sparkCount"]), "\(tag) spark count")

                for (dark, key) in [(false, "light"), (true, "dark")] {
                    let rows = Dashboard.modelRows(usage, dark: dark)
                    let wantRows = try #require(want[key] as? [[String: Any]])
                    #expect(rows.map(\.id) == wantRows.map { $0["id"] as? String ?? "" }, "\(tag) \(key) order")
                    for (row, w) in zip(rows, wantRows) {
                        #expect(row.name == w["name"] as? String, "\(tag) \(row.id) name")
                        #expect(row.color == w["color"] as? String, "\(tag) \(row.id) colour")
                        #expect(Fmt.money(row.cost) == w["costText"] as? String, "\(tag) \(row.id) cost")
                        #expect(JS.toFixed(row.sharePct, 0) + "%" == w["pctText"] as? String, "\(tag) \(row.id) share")
                        #expect(row.detail == w["detail"] as? [String], "\(tag) \(row.id) detail")
                    }
                }

                let tiles = Dashboard.toolTiles(usage)
                let wantTiles = try #require(want["tiles"] as? [[String: Any]])
                #expect(tiles.count == 6, "\(tag) six tiles")
                #expect(tiles.map(\.name) == wantTiles.map { $0["name"] as? String ?? "" }, "\(tag) tile order")
                for (t, w) in zip(tiles, wantTiles) {
                    #expect(Tools.displayName(t.name) == w["label"] as? String, "\(tag) \(t.name) label")
                    #expect(t.calls == dbl(w["calls"]), "\(tag) \(t.name) calls")
                    #expect(Dashboard.toolIcon(t.name).rawValue == w["icon"] as? String, "\(tag) \(t.name) icon")
                    #expect(Tools.color(t.name) == w["color"] as? String, "\(tag) \(t.name) colour")
                }
                #expect(tiles.reduce(0) { $0 + $1.calls } == dbl(want["toolTotal"]), "\(tag) tool total")

                let limits = try #require(want["limits"] as? [String: Any])
                let claude = Dashboard.claudeLimits(snap.claudeQuota, now: now)
                let codex = Dashboard.codexLimits(snap.codexQuota, log: usage?.codexQuota, now: now)
                for (row, key) in [(claude, "claude"), (codex, "codex")] {
                    if let w = limits[key] as? [String: Any] {
                        #expect(row != nil, "\(tag) \(key) row")
                        #expect(row?.fivePct == dbl(w["five"]), "\(tag) \(key) 5h")
                        #expect(row?.weeklyPct == dbl(w["weekly"]), "\(tag) \(key) weekly")
                        #expect(row?.staleAsOf == w["staleAsOf"] as? String, "\(tag) \(key) stale")
                    } else {
                        #expect(row == nil, "\(tag) no \(key) row")
                    }
                }

                let note = Dashboard.priceNote(usage?.priceHealth, now: now)
                if let w = want["priceNote"] as? [String: Any] {
                    #expect(note?.text == w["text"] as? String, "\(tag) price note")
                    #expect(note?.warn == w["warn"] as? Bool, "\(tag) price warn")
                } else {
                    #expect(note == nil, "\(tag) no price note")
                }
            }
        }
    }

    @Test func freshnessLine() throws {
        let v = try Fixtures.vectors
        let demo = try #require((v["envelope"] as? [String: Any])?["json"] as? [String: Any])
        for c in try #require(v["freshness"] as? [[String: Any]]) {
            var json = demo
            json["generatedAt"] = c["generatedAt"]
            let snap = try Snapshot.parse(try JSONSerialization.data(withJSONObject: json))
            let f = Dashboard.freshness(snap, error: syncError(c["error"]), now: date(ms: c["now"]))
            #expect(f.text == c["text"] as? String)
            #expect(f.alert == c["alert"] as? Bool, "\(f.text)")
        }
    }

    @Test func errorWording() throws {
        for c in try #require(try Fixtures.vectors["describeError"] as? [[String: Any]]) {
            let e = try #require(syncError(c["error"]))
            let d = e.describe(deviceName: c["deviceName"] as? String, hasSnapshot: c["hasSnapshot"] as? Bool ?? false)
            #expect(d.short == c["short"] as? String)
            #expect(d.long == c["long"] as? String)
        }
    }

    @Test func priceAge() throws {
        let now = Date(timeIntervalSince1970: 1_794_225_600)
        for c in try #require(try Fixtures.vectors["fmtAge"] as? [[String: Any]]) {
            #expect(Dashboard.fmtAge(c["iso"] as? String, now: now) == c["text"] as? String, "\(c["iso"] ?? "nil")")
        }
    }

    /** The widgets redraw exactly when a limit window rolls over. */
    @Test func widgetRedrawMoments() throws {
        let fx = try Fixtures.json("sync_envelope_vector")
        let snap = try Snapshot.parse(try JSONSerialization.data(withJSONObject: try #require(fx["json"])))
        // Claude 5h resets 11:50:00.6Z and Codex 5h 12:59:30Z; the weekly ones two days later.
        let t0 = try #require(ISODate.parse("2026-11-09T08:05:00Z"))
        #expect(Dashboard.changeMoments(snap, range: .today, after: t0, within: 15 * 60).isEmpty)
        let both = Dashboard.changeMoments(snap, range: .today, after: t0, within: 12 * 3600)
        #expect(both.count == 2)
        let reset = try #require(both.first)
        #expect(abs(reset.timeIntervalSince1970 - 1_794_225_001.612317) < 1e-3)
        #expect(Dashboard.claudeLimits(snap.claudeQuota, now: reset.addingTimeInterval(-2))?.fivePct == 25)
        #expect(Dashboard.claudeLimits(snap.claudeQuota, now: reset)?.fivePct == 0)
        #expect(Dashboard.changeMoments(snap, range: .today, after: reset, within: 3600).isEmpty)
        #expect(Dashboard.changeMoments(nil, range: .today, after: t0, within: 12 * 3600).isEmpty)
    }

    /** "as of …" only when the Mac really stopped, or the widget itself went unrefreshed. */
    @Test func widgetStaleness() throws {
        let fx = try Fixtures.json("sync_envelope_vector")
        let snap = try Snapshot.parse(try JSONSerialization.data(withJSONObject: try #require(fx["json"])))
        let gen = try #require(ISODate.parse("2026-11-09T08:00:00Z"))
        func stale(checkedAfter c: TimeInterval, shownAfter n: TimeInterval) -> Date? {
            Dashboard.widgetStaleSince(snap, checkedAt: gen.addingTimeInterval(c), now: gen.addingTimeInterval(n))
        }
        // A heartbeat-old snapshot stays "fresh" between the widget's own refreshes…
        #expect(stale(checkedAfter: 9 * 60, shownAfter: 9 * 60) == nil)
        #expect(stale(checkedAfter: 9 * 60, shownAfter: 60 * 60) == nil)
        #expect(stale(checkedAfter: 15 * 60, shownAfter: 16 * 60) == nil)
        // …but not when it was already old at the check, or after two hours without a refresh.
        #expect(stale(checkedAfter: 15 * 60 + 1, shownAfter: 15 * 60 + 1) == gen)
        #expect(stale(checkedAfter: 9 * 60, shownAfter: 9 * 60 + 2 * 3600 + 1) == gen)
        #expect(stale(checkedAfter: 9 * 60, shownAfter: 9 * 60 + 2 * 3600) == nil)
    }
}
