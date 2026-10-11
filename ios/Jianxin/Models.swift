import Foundation

struct MailAccount: Codable, Identifiable, Hashable {
    var id = UUID().uuidString
    var email = ""
    var displayName = ""
    var username = ""
    var credential = ""
    var oauth2 = false
    var imapHost = ""
    var imapPort = 993
    var imapStartTLS = false
    var smtpHost = ""
    var smtpPort = 465
    var smtpStartTLS = false
    var imapUsername: String?
    var smtpUsername: String?
    var folderOverrides: [String:String]?
    var title: String { displayName.isEmpty ? email : displayName }
    var login: String { username.isEmpty ? email : username }
    var imapLogin: String { imapUsername ?? login }
    var smtpLogin: String { smtpUsername ?? login }
    var importIdentity: String { email.lowercased()+"\n"+imapHost.lowercased()+"\n"+imapLogin }
    static func merge(_ incoming:[MailAccount],into existing:[MailAccount])->[MailAccount] {
        var next=existing
        for var account in incoming {
            if let index=next.firstIndex(where:{$0.importIdentity==account.importIdentity}){account.id=next[index].id;next[index]=account}
            else{next.append(account)}
        }
        return next
    }
    func validate() throws {
        guard MailAddress.valid(email), !credential.isEmpty, [imapHost, smtpHost].allSatisfy({ !$0.isEmpty && !$0.contains(where: { $0.isWhitespace || $0.isNewline }) }),
              (1...65535).contains(imapPort), (1...65535).contains(smtpPort),
              [login,imapLogin,smtpLogin].allSatisfy({!$0.isEmpty && !$0.contains(where:{$0.isNewline || $0 == "\u{0}"})}),
              !credential.contains("\u{0}"), !(folderOverrides?.values.contains(where:{$0.isEmpty || $0.contains(where:{$0.isNewline})}) ?? false)
        else { throw MailFailure.message("检查邮箱地址、授权码和服务器设置") }
    }
    mutating func preset(_ name: String) {
        let hosts: [String: (String,String)] = ["QQ":("imap.qq.com","smtp.qq.com"),"163":("imap.163.com","smtp.163.com"),
            "126":("imap.126.com","smtp.126.com"),"Gmail":("imap.gmail.com","smtp.gmail.com"),"阿里云":("imap.aliyun.com","smtp.aliyun.com")]
        if let pair = hosts[name] { imapHost=pair.0; smtpHost=pair.1; imapPort=993; smtpPort=465; imapStartTLS=false; smtpStartTLS=false }
    }
}
struct MailFolder: Codable, Identifiable, Hashable {
    var path: String
    var name: String
    var role: String
    var id: String { path }
    var title: String { ["inbox":"收件箱","sent":"已发送","drafts":"草稿箱","trash":"已删除","spam":"垃圾邮件","archive":"归档"][role] ?? name }
}
struct MailAddress: Codable, Hashable {
    var name = ""
    var email: String
    var title: String { name.isEmpty ? email : name }
    static func valid(_ email: String) -> Bool {
        email.range(of: "^[^\\s<>@,;]+@[^\\s<>@,;]+$", options: .regularExpression) != nil
    }
    static func parseList(_ text: String) -> [MailAddress] {
        // Split commas only outside quoted display names and angle brackets.
        var chunks: [String] = [], part = "", quoted = false, angle = 0, escape = false
        for c in text {
            if escape { part.append(c); escape=false; continue }
            if c == "\\" && quoted { escape=true; part.append(c); continue }
            if c == "\"" { quoted.toggle() }
            if c == "<" && !quoted { angle += 1 }
            if c == ">" && !quoted { angle = max(0,angle-1) }
            if (c == "," || c == ";" || c == "，" || c == "；") && !quoted && angle == 0 { chunks.append(part); part="" } else { part.append(c) }
        }
        chunks.append(part)
        return chunks.compactMap { chunk in
            let raw = chunk.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !raw.isEmpty else { return nil }
            if let start=raw.lastIndex(of:"<"), let end=raw[start...].firstIndex(of:">") {
                return MailAddress(name: MIME.decodeHeader(String(raw[..<start]).trimmingCharacters(in: CharacterSet(charactersIn:" \""))), email:String(raw[raw.index(after:start)..<end]))
            }
            return MailAddress(email: raw)
        }
    }
}
struct MailAttachment: Codable, Identifiable, Hashable {
    var id: String
    var filename: String
    var mimeType: String
    var size: Int
    var data: Data?
}
struct MailMessage: Codable, Identifiable, Hashable {
    var accountId: String
    var folder: String
    var validity: String
    var uid: String
    var subject = ""
    var from: [MailAddress] = []
    var to: [MailAddress] = []
    var cc: [MailAddress] = []
    var replyTo: [MailAddress] = []
    var messageId = ""
    var references: [String] = []
    var date = Date()
    var isRead = false
    var isStarred = false
    var body: String?
    var attachments: [MailAttachment] = []
    var id: String { "\(accountId):\(folder):\(validity):\(uid)" }
    var sender: String { from.first?.title ?? "未知发件人" }
    var title: String { subject.isEmpty ? "（无主题）" : subject }
}
struct ComposeDraft: Codable, Identifiable, Equatable {
    var id = UUID().uuidString
    var accountId: String
    var to = ""
    var cc = ""
    var bcc = ""
    var subject = ""
    var text = ""
    var inReplyTo = ""
    var references: [String] = []
    var attachments: [MailAttachment] = []
    var delivery = "draft" // draft, sending, sent, failed, uncertain
    var error = ""
    var date = Date()
    var title: String { subject.isEmpty ? "未命名草稿" : subject }
    var hasContent: Bool { !to.isEmpty || !text.isEmpty || !subject.isEmpty || !attachments.isEmpty }
    var recipients: [MailAddress] { MailAddress.parseList(to + "," + cc + "," + bcc) }
    func validate() throws {
        guard !recipients.isEmpty, recipients.allSatisfy({ MailAddress.valid($0.email) }), !subject.contains(where: { $0.isNewline }),
              ([to,cc,bcc,inReplyTo]+references).allSatisfy({ !$0.contains(where: { $0.isNewline }) }) else { throw MailFailure.message("检查收件人地址和主题") }
    }
}
enum MailFailure: LocalizedError {
    case message(String), uncertain
    var errorDescription: String? {
        switch self { case .message(let text): return text
        case .uncertain: return "服务器可能已收到邮件。先检查已发送邮件，再决定是否重发，避免重复发送。" }
    }
}
