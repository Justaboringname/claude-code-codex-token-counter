// Number and name formatting, ported from src/format.ts and src/sync/age.ts.
// Rounding follows JavaScript (toFixed rounds exact ties away from zero,
// Math.round rounds ties up) so the phone shows the same digits as the web
// app; Tests/FormatTests checks this against vectors from the TypeScript.
import Foundation

enum JS {
    /** Number.prototype.toFixed for |x| < 1e21. */
    static func toFixed(_ x: Double, _ digits: Int) -> String {
        if !x.isFinite { return x.isNaN ? "NaN" : (x > 0 ? "Infinity" : "-Infinity") }
        // Like JavaScript: any x < 0 prints a "-" (−0.4 → "-0"), −0 does not.
        let neg = x < 0
        let a = abs(x)
        // printf rounds the exact binary value correctly but breaks exact ties
        // to even; JavaScript picks the larger magnitude. Detect exact ties.
        let exact = String(format: "%.80f", a)
        var s = String(format: "%.\(digits)f", a)
        if let dot = exact.firstIndex(of: ".") {
            let frac = exact[exact.index(after: dot)...]
            if frac.count > digits {
                let rest = frac.dropFirst(digits)
                if rest.first == "5", rest.dropFirst().allSatisfy({ $0 == "0" }) {
                    let scale = pow(10.0, Double(digits))
                    let up = (a * scale).rounded(.down) + 1
                    s = String(format: "%.\(digits)f", up / scale)
                }
            }
        }
        return neg ? "-" + s : s
    }

    /** Math.round: ties toward +∞. */
    static func round(_ x: Double) -> Double { (x + 0.5).rounded(.down) }

    private static let grouped: NumberFormatter = {
        let f = NumberFormatter()
        f.locale = Locale(identifier: "en_US")
        f.numberStyle = .decimal
        f.maximumFractionDigits = 3
        return f
    }()

    /** Number.prototype.toLocaleString('en-US') for the integers we show. */
    static func localeString(_ x: Double) -> String {
        grouped.string(from: NSNumber(value: x)) ?? String(x)
    }
}

enum Fmt {
    static func money(_ n: Double, decimals: Int = 2) -> String {
        if n >= 1000 { return "$" + JS.localeString(JS.round(n)) }
        return "$" + JS.toFixed(n, decimals)
    }

    /** Token counts arrive in thousands. */
    static func tokens(_ k: Double) -> String {
        if k >= 1000 { return JS.toFixed(k / 1000, 1) + "M" }
        if k >= 1 { return JS.toFixed(k, 0) + "K" }
        return JS.toFixed(k * 1000, 0)
    }

    static func credits(_ n: Double) -> String {
        if n >= 1_000_000 { return JS.toFixed(n / 1_000_000, 2) + "M" }
        if n >= 10_000 { return JS.toFixed(n / 1000, 0) + "K" }
        if n >= 1000 { return JS.toFixed(n / 1000, 1) + "K" }
        return JS.localeString(JS.round(n))
    }

    static func pctOf(_ used: Double, _ limit: Double) -> Double {
        guard limit > 0 else { return 0 }
        return min(100, used / limit * 100)
    }

    /** "just now", "1 min ago", "12 min ago", "3 h ago", "2 days ago". */
    static func relativeAge(_ then: Date, now: Date = Date()) -> String {
        let s = max(0, now.timeIntervalSince(then))
        if !s.isFinite { return "at an unknown time" }
        if s < 45 { return "just now" }
        let m = JS.round(s / 60)
        if m < 60 { return "\(Int(max(1, m))) min ago" }
        let h = (m / 60).rounded(.down)
        if h < 48 { return "\(Int(h)) h ago" }
        return "\(Int((h / 24).rounded(.down))) days ago"
    }
}

// MARK: models and tools

struct ModelMeta: Equatable, Sendable {
    var name: String
    var color: String // #RRGGBB
    var family: String // fable | opus | sonnet | haiku | codex | other
}

enum Models {
    private static let familyColors: [String: String] = [
        "fable": "#B8532F",
        "opus": "#C86F49",
        "sonnet": "#D88C62",
        "haiku": "#E8A87C",
        "other": "#D97757",
    ]
    private static let codexBlues = ["#85B5E0", "#6FA6D9", "#5893CD", "#437FBD", "#336CAA", "#265A94"]
    private static let codexTiers: [(String, String)] = [("-ultrafast", "Ultrafast"), ("-flex", "Flex"), ("-fast", "Fast")]

    /** Anything that isn't Claude is an OpenAI/Codex model. */
    static func isCodex(_ id: String) -> Bool {
        let c = id.lowercased()
        return !(c.hasPrefix("claude") || c.contains(/opus|sonnet|haiku|fable/))
    }

    private static func titleCase(_ w: Substring) -> String { w.prefix(1).uppercased() + w.dropFirst() }

    private static func codexMeta(_ id: String) -> ModelMeta {
        var c = id
        var tier = ""
        for (suffix, label) in codexTiers where c.hasSuffix(suffix) {
            c.removeLast(suffix.count)
            tier = label
            break
        }
        let long = c.hasSuffix("-long")
        if long { c.removeLast("-long".count) }
        let tags = (long ? " 272K+" : "") + (tier.isEmpty ? "" : " \(tier)")
        if c == "codex-auto-review" { return ModelMeta(name: "Codex Review\(tags)", color: "#8CA8C4", family: "codex") }
        if let d = c.firstMatch(of: /^gpt-daybreak-([a-z]+)/) {
            return ModelMeta(name: "Daybreak \(titleCase(d.1))\(tags)", color: codexBlues[5], family: "codex")
        }
        let m = c.firstMatch(of: /gpt-(\d+)(?:\.(\d+))?/)
        let minor = m?.2.flatMap { Int($0) } ?? 0
        let version = m.map { $0.2 != nil ? "\($0.1).\($0.2!)" : String($0.1) } ?? ""
        var name = m != nil ? "GPT-\(version)" : (c == "codex" ? "Codex" : c)
        let variant = c.firstMatch(of: /-(astra|sol|terra|luna|cyber|pro|mini|nano)(?:-|$)/)
        if m != nil && c.contains("codex") { name += " Codex" }
        if m != nil, let v = variant { name += " \(titleCase(v.1))" }
        if c.contains("-max") { name += " Max" }
        name += tags
        let color = codexBlues[min(max(minor, 0), codexBlues.count - 1)]
        return ModelMeta(name: name, color: color, family: "codex")
    }

    static func meta(_ canonical: String) -> ModelMeta {
        let c = canonical.lowercased()
        if isCodex(c) { return codexMeta(c) }
        let m = c.firstMatch(of: /(fable|opus|sonnet|haiku)-(\d+)(?:-(\d+))?/)
        let u = m == nil ? c.firstMatch(of: /^claude-([a-z]+)(?:-(\d+))?(?:-(\d+))?/) : nil
        let family = m.map { String($0.1) } ?? "other"
        let word = m.map { String($0.1) } ?? u.map { String($0.1) } ?? ""
        let major = m.map { String($0.2) } ?? u?.2.map(String.init)
        let minor = m?.3.map(String.init) ?? u?.3.map(String.init)
        let version = major.map { mj in minor.map { "\(mj).\($0)" } ?? mj } ?? ""
        let familyLabel = word.isEmpty ? "Claude" : word.prefix(1).uppercased() + word.dropFirst()
        let base = version.isEmpty ? familyLabel : "\(familyLabel) \(version)"
        let name = c.hasSuffix("-fast") ? "\(base) Fast" : base
        return ModelMeta(name: name, color: familyColors[family] ?? familyColors["other"]!, family: family)
    }

    /** Dark-appearance accent lift (#007AFF → #0A84FF style). Expects #RRGGBB. */
    static func liftForDark(_ hex: String, amount: Double = 0.28) -> String {
        guard hex.count == 7, hex.hasPrefix("#"), let n = Int(hex.dropFirst(), radix: 16) else { return hex }
        func lift(_ c: Int) -> Int { Int(JS.round(Double(c) + Double(255 - c) * amount)) }
        let (r, g, b) = (lift(n >> 16), lift((n >> 8) & 0xFF), lift(n & 0xFF))
        return String(format: "#%06x", (r << 16) | (g << 8) | b)
    }

    // Costs: the Mac always sends `cost`; this table is only the web app's
    // fallback for a model without one.
    private static func pricing(_ id: String) -> (input: Double, output: Double, cacheWrite: Double, cacheRead: Double) {
        let c = id.lowercased()
        if isCodex(c) {
            func p(_ i: Double, _ o: Double, _ r: Double) -> (Double, Double, Double, Double) { (i, o, 0, r) }
            if c.hasPrefix("gpt-6-astra") { return p(10, 50, 1) }
            if c.hasPrefix("gpt-6.1-sol") { return p(2, 10, 0.1) }
            if c.hasPrefix("gpt-6-sol") { return p(2, 10, 0.2) }
            if c.hasPrefix("gpt-6-luna") { return p(0.1, 0.5, 0.01) }
            if c.contains("-cyber") { return p(12.5, 75, 1.25) }
            if c.contains("5.6-terra") { return p(2, 12, 0.2) }
            if c.contains("5.6-luna") { return p(0.2, 1.2, 0.02) }
            if c == "gpt-5.6" || c.contains("5.6-sol") { return p(4, 20, 0.4) }
            if c.contains("5.5-pro") || c.contains("5.4-pro") { return p(30, 180, 0) }
            if c.contains("5.5") { return p(5, 30, 0.5) }
            if c.contains("5.4-mini") { return p(0.75, 4.5, 0.075) }
            if c.contains("5.4-nano") { return p(0.2, 1.25, 0.02) }
            if c.contains("5.4") { return p(2.5, 15, 0.25) }
            if c.contains("5.2-pro") { return p(21, 168, 0) }
            if c.contains("5.2") || c.contains("5.3") { return p(1.75, 14, 0.175) }
            if c.hasPrefix("gpt-5.1-codex-mini") { return p(0.25, 2, 0.025) }
            if c == "codex-mini-latest" { return p(1.5, 6, 0.375) }
            if c.hasPrefix("gpt-5-pro") { return p(15, 120, 0) }
            if c.hasPrefix("gpt-5-mini") { return p(0.25, 2, 0.025) }
            if c.hasPrefix("gpt-5-nano") { return p(0.05, 0.4, 0.005) }
            if c == "gpt-5" || c.hasPrefix("gpt-5-") || c == "gpt-5.1" || c.hasPrefix("gpt-5.1-") || c == "codex"
                || c == "codex-auto-review" { return p(1.25, 10, 0.125) }
            return p(10, 50, 1)
        }
        let (input, output, cacheRead): (Double, Double, Double) =
            c.contains("fable-5-1") ? (10, 50, 0.25)
            : c.contains("fable") ? (10, 50, 1)
            : c.contains("opus-5-5") ? (4, 20, 0.2)
            : c.contains("opus") ? (5, 25, 0.5)
            : c.contains("haiku-5-5") ? (0.1, 0.5, 0.01)
            : c.contains("haiku") ? (1, 5, 0.1)
            : c.contains("sonnet-5-5") ? (2, 10, 0.1)
            : c.contains("sonnet-5") ? (2, 10, 0.2)
            : (3, 15, 0.3)
        let f: Double = !c.contains("-fast") ? 1 : (c.contains("opus-5") || c.contains("opus-4-8")) ? 2 : 6
        return (input * f, output * f, input * 1.25 * f, cacheRead * f)
    }

    static func cost(_ id: String, _ u: ModelUsage) -> Double {
        if let c = u.cost { return c }
        let p = pricing(id)
        return (u.inK * p.input + u.outK * p.output + u.cacheWriteK * p.cacheWrite + u.cacheReadK * p.cacheRead) / 1000
    }

    static func rawTokens(_ u: ModelUsage) -> Double { u.inK + u.outK + u.cacheWriteK + u.cacheReadK }
}

enum Tools {
    static let colors: [String: String] = [
        "Read": "#D97757", "Edit": "#B8532F", "Write": "#B8532F", "MultiEdit": "#B8532F",
        "Bash": "#8B6F47", "BashOutput": "#8B6F47", "Grep": "#E8A87C",
        "WebFetch": "#6B8E4E", "WebSearch": "#6B8E4E", "Glob": "#A68C6C",
        "shell": "#437FBD", "apply_patch": "#5893CD", "update_plan": "#6FA6D9", "web_search": "#336CAA",
    ]
    static func color(_ name: String) -> String { colors[name] ?? "#8B6F47" }

    private static let codexNames = ["shell": "Shell", "apply_patch": "Apply Patch", "update_plan": "Plan", "web_search": "Web Search"]

    /** MCP ids ("mcp__<Server>__<tool>") show just the tool segment. */
    static func displayName(_ name: String) -> String {
        if let n = codexNames[name] { return n }
        guard name.hasPrefix("mcp__") else { return name }
        let rest = name.dropFirst(5)
        if let r = rest.range(of: "__") { return String(rest[r.upperBound...]) }
        return String(rest)
    }
}
