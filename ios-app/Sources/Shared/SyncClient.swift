// The three server RPCs (tc_head / tc_get; tc_put is the Mac's), like the
// web app's rpc(): POST /rest/v1/rpc/<name> with the publishable key, the
// channel and the bearer token in the body.
import Foundation

struct HeadInfo: Sendable, Equatable {
    var version: Int
    var updatedAt: String?
    var generatedAt: String?
    var device: String?
}

struct RequestInfo: Sendable, Equatable {
    /** When the server took the request (compare with a later head's updatedAt). */
    var requestedAt: String?
    /** Seconds since an uploading Mac last checked in; nil if none ever has. */
    var uploaderAgeSecs: Double?
}

struct GetInfo: Sendable {
    var head: HeadInfo
    var payload: String
}

struct SyncClient: Sendable {
    var keys: SyncKeys
    var timeout: TimeInterval = 20

    private static let session: URLSession = {
        let c = URLSessionConfiguration.ephemeral
        c.requestCachePolicy = .reloadIgnoringLocalCacheData
        c.urlCache = nil
        c.httpCookieStorage = nil
        c.waitsForConnectivity = false
        return URLSession(configuration: c)
    }()

    /** Current version + timestamps, no payload (cheap; polled). */
    func head() async throws -> HeadInfo {
        Self.asHead(try await rpc("tc_head"))
    }

    /** Version + the envelope. Throws SyncError(.empty) when nothing was uploaded yet. */
    func get() async throws -> GetInfo {
        let r = try await rpc("tc_get")
        guard let o = r as? [String: Any], let payload = o["payload"] as? String, !payload.isEmpty else {
            throw SyncError(kind: .empty, message: "Nothing has been uploaded yet.")
        }
        return GetInfo(head: Self.asHead(r), payload: payload)
    }

    /** Ask the uploading Mac for a fresh snapshot now (it checks every few
     *  seconds). A server from before this existed answers 404 → .network. */
    func request() async throws -> RequestInfo {
        let o = try await rpc("tc_request") as? [String: Any] ?? [:]
        return RequestInfo(requestedAt: o["requestedAt"] as? String, uploaderAgeSecs: JSONValue.number(o["uploaderAgeSecs"]))
    }

    /** get → open, as one call (first pairing). */
    func fetchSnapshot() async throws -> (snapshot: Snapshot, version: Int) {
        let g = try await get()
        return (try SyncProtocol.openEnvelope(g.payload, keys: keys), g.head.version)
    }

    private func rpc(_ name: String) async throws -> Any? {
        guard let url = URL(string: "\(keys.url)/rest/v1/rpc/\(name)") else {
            throw SyncError(kind: .badCode, message: "Not a Token Counter pairing code (invalid server URL).")
        }
        var req = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: timeout)
        req.httpMethod = "POST"
        req.setValue(keys.apiKey, forHTTPHeaderField: "apikey")
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = try JSONSerialization.data(withJSONObject: ["p_channel": keys.channel, "p_token": keys.token])
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await Self.session.data(for: req)
        } catch let e as URLError where e.code == .timedOut {
            throw SyncError(kind: .network, message: "The sync server did not answer in time.")
        } catch is CancellationError {
            throw CancellationError()
        } catch {
            throw SyncError(kind: .network, message: "Can't reach the sync server.")
        }
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        let json = data.isEmpty ? nil : try? JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed])
        if status == 401 || status == 403 {
            let code = (json as? [String: Any])?["code"] as? String
            throw SyncError(
                kind: .forbidden,
                message: code == "42501"
                    ? "This pairing code isn't registered on the server."
                    : "The sync server rejected this pairing code.",
                status: status
            )
        }
        guard (200..<300).contains(status) else {
            throw SyncError(kind: .network, message: "The sync server answered \(status).", status: status)
        }
        if json == nil && !data.isEmpty {
            throw SyncError(kind: .network, message: "The sync server sent an unreadable reply.", status: status)
        }
        return json is NSNull ? nil : json
    }

    private static func asHead(_ v: Any?) -> HeadInfo {
        let o = v as? [String: Any] ?? [:]
        let version = JSONValue.number(o["version"]).map { Int(exactly: $0.rounded(.towardZero)) ?? 0 } ?? 0
        return HeadInfo(
            version: version,
            updatedAt: o["updatedAt"] as? String,
            generatedAt: o["generatedAt"] as? String,
            device: o["device"] as? String
        )
    }
}
