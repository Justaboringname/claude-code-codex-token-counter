// The synced usage snapshot (schema 1) and its validation, the Swift side of
// src/sync/snapshot.ts + src/types.ts.
//
// A snapshot is authentic (AES-GCM under the pairing code) but not trusted to
// be well-formed: a newer Mac may change a field's type. So, like the web app:
//   - a newer schema major is refused (SyncError.newerSchema);
//   - fields the UI reads are coerced (numbers → finite or 0, strings, arrays
//     of the right items), optional ones dropped when invalid;
//   - unknown fields and missing ranges are fine.
// Token counts in ModelUsage and Usage.totalTokens are in THOUSANDS.
import Foundation

enum UsageRange: String, CaseIterable, Sendable, Identifiable {
    case session, today, week, month, all
    var id: String { rawValue }
    var label: String {
        switch self {
        case .session: "Session"
        case .today: "Today"
        case .week: "Week"
        case .month: "Month"
        case .all: "All"
        }
    }
}

struct LimitWindow: Sendable, Equatable {
    var usedPct: Double
    var resetsAt: String?
}

/** ClaudeQuota / CodexLiveQuota (same core shape). */
struct LiveQuota: Sendable, Equatable {
    var fiveHour: LimitWindow?
    var weekly: LimitWindow?
    var status: String // ok | stale | no_token | error
    var message: String?
    var fetchedAt: String
    var subscription: String? // Claude
    var planType: String? // Codex
    var resetCredits: Double? // Codex
}

/** Usage.codexQuota: the rollout-log fallback. `*ResetsAt` are unix seconds. */
struct LogQuota: Sendable, Equatable {
    var asOf: String
    var primaryUsedPct: Double
    var primaryWindowMinutes: Double
    var primaryResetsAt: Double?
    var secondaryUsedPct: Double
    var secondaryWindowMinutes: Double
    var secondaryResetsAt: Double?
    var planType: String?
}

struct PriceHealth: Sendable, Equatable {
    var status: String // fresh | aging | stale | error | builtin
    var fetchedAt: String?
    var lastError: String?
    var lastErrorAt: String?
}

struct ModelUsage: Sendable, Equatable {
    var inK: Double
    var outK: Double
    var cacheWriteK: Double
    var cacheReadK: Double
    var cost: Double?
    var priceSource: String?
}

struct ToolCount: Sendable, Equatable {
    var name: String
    var calls: Double
    var tokens: Double
}

struct Usage: Sendable, Equatable {
    /** In the order the JSON had them. */
    var byModel: [(id: String, usage: ModelUsage)]
    var byTool: [ToolCount]
    var sparkline: [Double]
    var sparklineCodex: [Double]?
    var codexQuota: LogQuota?
    var sparkLeft: String
    var sparkRight: String
    var rangeLabel: String
    var totalCost: Double
    var totalCostEst: Double?
    var totalTokens: Double
    var deltaPct: Double?
    var creditUsed5h: Double
    var creditUsedWeek: Double
    var creditLimit5h: Double
    var creditLimitWeek: Double
    var planLabel: String
    var planId: String
    var sessionStart: String?
    var tokens5h: Double
    var tokensWeek: Double
    var priorityEvents5h: Double
    var activeBlockStart: String?
    var tokenLimit5h: Double
    var priceHealth: PriceHealth?

    static func == (a: Usage, b: Usage) -> Bool {
        a.byModel.map(\.id) == b.byModel.map(\.id) && a.byModel.map(\.usage) == b.byModel.map(\.usage)
            && a.byTool == b.byTool && a.sparkline == b.sparkline && a.sparklineCodex == b.sparklineCodex
            && a.codexQuota == b.codexQuota && a.sparkLeft == b.sparkLeft && a.sparkRight == b.sparkRight
            && a.rangeLabel == b.rangeLabel && a.totalCost == b.totalCost && a.totalCostEst == b.totalCostEst
            && a.totalTokens == b.totalTokens && a.deltaPct == b.deltaPct && a.creditUsed5h == b.creditUsed5h
            && a.creditUsedWeek == b.creditUsedWeek && a.creditLimit5h == b.creditLimit5h
            && a.creditLimitWeek == b.creditLimitWeek && a.planLabel == b.planLabel && a.planId == b.planId
            && a.sessionStart == b.sessionStart && a.tokens5h == b.tokens5h && a.tokensWeek == b.tokensWeek
            && a.priorityEvents5h == b.priorityEvents5h && a.activeBlockStart == b.activeBlockStart
            && a.tokenLimit5h == b.tokenLimit5h && a.priceHealth == b.priceHealth
    }
}

struct Snapshot: Sendable, Equatable {
    var schema: Double
    var generatedAt: String
    var deviceName: String
    var appVersion: String?
    var plan: String?
    var usage: [UsageRange: Usage]
    var claudeQuota: LiveQuota?
    var codexQuota: LiveQuota?
    /** The JSON this was parsed from (what the cache stores). */
    var source: Data

    static let schemaVersion = 1
    /** Longer series than any real range has; keeps a hostile snapshot from
     *  becoming a million-point path. */
    static let maxSeries = 2000

    static func == (a: Snapshot, b: Snapshot) -> Bool { a.source == b.source }

    /** Parse + validate decrypted JSON bytes. Throws SyncError(.badSnapshot). */
    static func parse(_ json: Data) throws -> Snapshot {
        let any: Any
        do {
            guard String(data: json, encoding: .utf8) != nil else { throw Gzip.FormatError() }
            any = try JSONSerialization.jsonObject(with: json, options: [.fragmentsAllowed])
        } catch {
            throw SyncError(kind: .badSnapshot, message: "The synced data could not be read (not gzip’d JSON).")
        }
        return try validate(any, source: json)
    }

    static func validate(_ v: Any, source: Data) throws -> Snapshot {
        func shape(_ why: String) -> SyncError {
            SyncError(kind: .badSnapshot, message: "The synced data is not a usage snapshot (\(why)).")
        }
        guard let o = v as? [String: Any] else { throw shape("not an object") }
        guard let schema = JSONValue.number(o["schema"]), schema >= 1 else { throw shape("no schema") }
        // Compared as a Double: Int(_:) traps on a value past Int.max.
        let major = schema.rounded(.down)
        if major > Double(schemaVersion) {
            let shown = Int(exactly: major).map(String.init) ?? String(format: "%.0f", major)
            throw SyncError(
                kind: .badSnapshot,
                message: "The main Mac runs a newer Token Counter (data format \(shown)).",
                newerSchema: true
            )
        }
        guard let usageObj = o["usage"] as? [String: Any] else { throw shape("no usage") }
        guard let generatedAt = o["generatedAt"] as? String else { throw shape("no generatedAt") }

        let device = o["device"] as? [String: Any] ?? [:]
        let rawName = device["name"] as? String
        let name = (rawName?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ?? true) ? "Main Mac" : rawName!

        var usage: [UsageRange: Usage] = [:]
        for r in UsageRange.allCases {
            if let u = usageObj[r.rawValue] as? [String: Any] { usage[r] = parseUsage(u) }
        }
        return Snapshot(
            schema: schema,
            generatedAt: generatedAt,
            deviceName: name,
            appVersion: device["appVersion"] as? String,
            plan: o["plan"] as? String,
            usage: usage,
            claudeQuota: liveQuota(o["claudeQuota"]),
            codexQuota: liveQuota(o["codexQuota"]),
            source: source
        )
    }

    // MARK: field helpers (the web app's num/str/series)

    private static func num(_ v: Any?) -> Double { JSONValue.number(v) ?? 0 }
    private static func str(_ v: Any?) -> String { (v as? String) ?? "" }

    private static func series(_ v: Any?) -> [Double]? {
        guard let a = v as? [Any] else { return nil }
        return a.suffix(maxSeries).map { JSONValue.number($0) ?? 0 }
    }

    private static func limitWindow(_ v: Any?) -> LimitWindow? {
        guard let o = v as? [String: Any] else { return nil }
        return LimitWindow(usedPct: num(o["usedPct"]), resetsAt: o["resetsAt"] as? String)
    }

    private static let quotaStatus: Set<String> = ["ok", "stale", "no_token", "error"]

    private static func liveQuota(_ v: Any?) -> LiveQuota? {
        guard let o = v as? [String: Any] else { return nil }
        let status = (o["status"] as? String).flatMap { quotaStatus.contains($0) ? $0 : nil } ?? "error"
        return LiveQuota(
            fiveHour: limitWindow(o["fiveHour"]),
            weekly: limitWindow(o["weekly"]),
            status: status,
            message: o["message"] as? String,
            fetchedAt: str(o["fetchedAt"]),
            subscription: o["subscription"] as? String,
            planType: o["planType"] as? String,
            resetCredits: JSONValue.number(o["resetCredits"])
        )
    }

    private static func logQuota(_ v: Any?) -> LogQuota? {
        guard let o = v as? [String: Any] else { return nil }
        return LogQuota(
            asOf: str(o["asOf"]),
            primaryUsedPct: num(o["primaryUsedPct"]),
            primaryWindowMinutes: num(o["primaryWindowMinutes"]),
            primaryResetsAt: JSONValue.number(o["primaryResetsAt"]),
            secondaryUsedPct: num(o["secondaryUsedPct"]),
            secondaryWindowMinutes: num(o["secondaryWindowMinutes"]),
            secondaryResetsAt: JSONValue.number(o["secondaryResetsAt"]),
            planType: o["planType"] as? String
        )
    }

    private static let priceStatus: Set<String> = ["fresh", "aging", "stale", "error", "builtin"]

    private static func priceHealth(_ v: Any?) -> PriceHealth? {
        guard let o = v as? [String: Any], let s = o["status"] as? String, priceStatus.contains(s) else { return nil }
        return PriceHealth(
            status: s,
            fetchedAt: o["fetchedAt"] as? String,
            lastError: o["lastError"] as? String,
            lastErrorAt: o["lastErrorAt"] as? String
        )
    }

    private static func modelUsage(_ v: Any?) -> ModelUsage? {
        guard let o = v as? [String: Any] else { return nil }
        return ModelUsage(
            inK: num(o["in"]),
            outK: num(o["out"]),
            cacheWriteK: num(o["cacheWrite"]),
            cacheReadK: num(o["cacheRead"]),
            cost: JSONValue.number(o["cost"]),
            priceSource: o["priceSource"] as? String
        )
    }

    /** byModel comes back sorted by id (JSONSerialization loses key order);
     *  the UI orders models by cost anyway. */
    private static func parseUsage(_ o: [String: Any]) -> Usage {
        var models: [(id: String, usage: ModelUsage)] = []
        if let bm = o["byModel"] as? [String: Any] {
            for (id, m) in bm {
                if let mu = modelUsage(m) { models.append((id, mu)) }
            }
            models.sort { $0.id < $1.id }
        }
        var tools: [ToolCount] = []
        if let bt = o["byTool"] as? [Any] {
            for t in bt {
                guard let to = t as? [String: Any], let name = to["name"] as? String, !name.isEmpty else { continue }
                tools.append(ToolCount(name: name, calls: num(to["calls"]), tokens: num(to["tokens"])))
            }
        }
        return Usage(
            byModel: models,
            byTool: tools,
            sparkline: series(o["sparkline"]) ?? [],
            sparklineCodex: series(o["sparklineCodex"]),
            codexQuota: logQuota(o["codexQuota"]),
            sparkLeft: str(o["sparkLeft"]),
            sparkRight: str(o["sparkRight"]),
            rangeLabel: str(o["rangeLabel"]),
            totalCost: num(o["totalCost"]),
            totalCostEst: JSONValue.number(o["totalCostEst"]),
            totalTokens: num(o["totalTokens"]),
            deltaPct: JSONValue.number(o["deltaPct"]),
            creditUsed5h: num(o["creditUsed5h"]),
            creditUsedWeek: num(o["creditUsedWeek"]),
            creditLimit5h: num(o["creditLimit5h"]),
            creditLimitWeek: num(o["creditLimitWeek"]),
            planLabel: str(o["planLabel"]),
            planId: str(o["planId"]),
            sessionStart: o["sessionStart"] as? String,
            tokens5h: num(o["tokens5h"]),
            tokensWeek: num(o["tokensWeek"]),
            priorityEvents5h: num(o["priorityEvents5h"]),
            activeBlockStart: o["activeBlockStart"] as? String,
            tokenLimit5h: num(o["tokenLimit5h"]),
            priceHealth: priceHealth(o["priceHealth"])
        )
    }
}
