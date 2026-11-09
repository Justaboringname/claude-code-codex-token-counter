import Foundation

/** RFC 3339 / ISO 8601 timestamps as the Mac writes them: `…:00Z`,
 *  `….906+00:00` and Python-style 6-digit microseconds `….612317+00:00`
 *  (ISO8601DateFormatter rejects some of these). A missing zone means local
 *  time, like JavaScript's Date. */
enum ISODate {
    static func parse(_ s: String?) -> Date? {
        guard let s else { return nil }
        let re = /^(\d{4})-(\d{2})-(\d{2})[T ](\d{2}):(\d{2})(?::(\d{2})(?:[.,](\d+))?)?(Z|z|[+-]\d{2}(?::?\d{2})?)?$/
        guard let m = s.trimmingCharacters(in: .whitespaces).wholeMatch(of: re) else { return nil }
        var c = DateComponents()
        c.year = Int(m.1)
        c.month = Int(m.2)
        c.day = Int(m.3)
        c.hour = Int(m.4)
        c.minute = Int(m.5)
        c.second = m.6.flatMap { Int($0) } ?? 0
        var cal = Calendar(identifier: .gregorian)
        if let zone = m.8 {
            let z = String(zone)
            if z == "Z" || z == "z" {
                cal.timeZone = TimeZone(secondsFromGMT: 0)!
            } else {
                let sign = z.hasPrefix("-") ? -1 : 1
                let digits = z.dropFirst().filter(\.isNumber)
                let hh = Int(digits.prefix(2)) ?? 0
                let mm = digits.count > 2 ? Int(digits.dropFirst(2)) ?? 0 : 0
                guard let tz = TimeZone(secondsFromGMT: sign * (hh * 3600 + mm * 60)) else { return nil }
                cal.timeZone = tz
            }
        } else {
            cal.timeZone = .current
        }
        guard let whole = cal.date(from: c), cal.component(.day, from: whole) == c.day else { return nil }
        guard let frac = m.7 else { return whole }
        let f = Double("0." + frac) ?? 0
        return whole.addingTimeInterval(f)
    }

    static func ms(_ s: String?) -> Double? {
        parse(s).map { $0.timeIntervalSince1970 * 1000 }
    }
}
