import AppKit
import UserNotifications

// This binary must run inside the signed Jianxin.app bundle.
let arguments = Array(CommandLine.arguments.dropFirst())
let helperMode = arguments.first?.hasPrefix("--") == true
let application = NSApplication.shared
application.setActivationPolicy(.accessory)
let center = UNUserNotificationCenter.current()

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}

func finish(_ status: String) {
    print(status)
    exit(0)
}

func permissionName(_ status: UNAuthorizationStatus) -> String {
    switch status {
    case .notDetermined: return "not_determined"
    case .denied: return "denied"
    case .authorized: return "authorized"
    case .provisional: return "provisional"
    case .ephemeral: return "ephemeral"
    @unknown default: return "unknown"
    }
}

func cli(_ args: [String]) {
    guard let executable = Bundle.main.url(forResource: "jianxin", withExtension: nil) else {
        fail("Bundled jianxin executable is missing")
    }
    let process = Process()
    process.executableURL = executable
    process.arguments = args
    process.currentDirectoryURL = FileManager.default.homeDirectoryForCurrentUser
    let logURL = FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent("Library/Application Support/jianxin/jianxin-app.log")
    try? FileManager.default.createDirectory(at: logURL.deletingLastPathComponent(), withIntermediateDirectories: true)
    if !FileManager.default.fileExists(atPath: logURL.path) {
        FileManager.default.createFile(atPath: logURL.path, contents: nil)
    }
    if let log = try? FileHandle(forWritingTo: logURL) {
        log.seekToEndOfFile()
        process.standardOutput = log
        process.standardError = log
    }
    do { try process.run() } catch { fail("Cannot start jianxin: \(error.localizedDescription)") }
}

final class AppDelegate: NSObject, NSApplicationDelegate, UNUserNotificationCenterDelegate {
    var openingFromNotification = false
    var allowsDesktopReopen = false
    var desktopProcess: Process?
    func applicationWillFinishLaunching(_ notification: Notification) {
        // Register before launch completes so a notification can cold-start the app.
        center.delegate = self
    }
    func applicationDidFinishLaunching(_ notification: Notification) {
        if helperMode {
            handleHelper()
            return
        }
        center.requestAuthorization(options: [.alert, .badge, .sound]) { _, _ in }
        let notificationLaunch = notification.userInfo?[NSApplication.launchUserNotificationUserInfoKey] != nil
        let defaultLaunch = notification.userInfo?[NSApplication.launchIsDefaultUserInfoKey] as? Bool ?? true
        // A notification launch is handled by didReceive; never start its desktop panel.
        guard defaultLaunch && !notificationLaunch && !openingFromNotification else { return }
        showDesktop()
    }
    func showDesktop() {
        allowsDesktopReopen = true
        guard let executable = Bundle.main.url(forResource: "jianxin-desktop", withExtension: nil) else {
            fail("Bundled jianxin-desktop executable is missing")
        }
        let process = Process()
        process.executableURL = executable
        // Reopening uses the desktop's instance lock and show request.
        process.arguments = ["--show"]
        process.currentDirectoryURL = FileManager.default.homeDirectoryForCurrentUser
        let ownsDesktop = desktopProcess == nil || desktopProcess?.isRunning == false
        if ownsDesktop {
            process.terminationHandler = { _ in
                DispatchQueue.main.async { NSApplication.shared.terminate(nil) }
            }
            desktopProcess = process
        }
        do { try process.run() } catch { fail("Cannot start desktop: \(error.localizedDescription)") }
    }
    @objc func openMail() { cli(["web", "--port", "0", "--idle-seconds", "1800", "--open"]) }
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        guard allowsDesktopReopen && !openingFromNotification else { return false }
        showDesktop()
        return true
    }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        return false
    }
    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification,
                                withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        completionHandler([.banner, .list, .sound])
    }
    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
                                withCompletionHandler completionHandler: @escaping () -> Void) {
        guard response.actionIdentifier == UNNotificationDefaultActionIdentifier else {
            completionHandler()
            return
        }
        DispatchQueue.main.async {
            self.openingFromNotification = true
            // This runs only in response to the user's notification click.
            let metadata = response.notification.request.content.userInfo
            if let account = metadata["account"] as? String, !account.isEmpty,
               let message = metadata["message_id"] as? String, !message.isEmpty {
                var args: [String] = []
                if let config = metadata["config"] as? String, !config.isEmpty {
                    args += ["--config", config]
                }
                args += ["--account", account, "open", "--", message]
                cli(args)
            } else {
                self.openMail()
            }
            completionHandler()
            // The Web child, Rust tray and launchd watcher outlive this wrapper.
            NSApplication.shared.terminate(nil)
        }
    }
    func handleHelper() {
        switch arguments.first {
        case "--notification-status":
            center.getNotificationSettings { settings in
                finish("{\"app_id\":\"com.jianxin.mail\",\"permission\":\"\(permissionName(settings.authorizationStatus))\"}")
            }
        case "--register-notifications":
            center.requestAuthorization(options: [.alert, .badge, .sound]) { granted, error in
                if let error = error { fail(error.localizedDescription) }
                if !granted { fail("简信通知未获授权：在系统设置 → 通知 → 简信中开启通知") }
                finish("{\"app_id\":\"com.jianxin.mail\",\"permission\":\"authorized\"}")
            }
        case "--notify":
            guard arguments.count == 2,
                  let data = arguments[1].data(using: .utf8),
                  let payload = try? JSONSerialization.jsonObject(with: data) as? [String: String] else {
                fail("Invalid notification payload")
            }
            center.getNotificationSettings { settings in
                guard settings.authorizationStatus == .authorized || settings.authorizationStatus == .provisional else {
                    fail("Notifications not authorized; run jianxin notifications --register")
                }
                let content = UNMutableNotificationContent()
                content.title = payload["title"] ?? "新邮件"
                content.subtitle = payload["subtitle"] ?? "简信"
                content.body = payload["body"] ?? ""
                content.threadIdentifier = payload["thread"] ?? ""
                // Retained by macOS, including notification-center / cold-start clicks.
                content.userInfo = Dictionary(uniqueKeysWithValues:
                    ["account", "message_id", "config"].compactMap { key in
                        payload[key].map { (key, $0) }
                    })
                let request = UNNotificationRequest(identifier: payload["id"] ?? UUID().uuidString,
                    content: content, trigger: nil)
                center.add(request) { error in
                    if let error = error { fail(error.localizedDescription) }
                    finish("{\"accepted\":true}")
                }
            }
        default: fail("Unknown helper command")
        }
    }
}

let delegate = AppDelegate()
application.delegate = delegate
application.run()
