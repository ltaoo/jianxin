import Foundation

struct IMAPRecord { var line: String; var literals: [Data] = [] }
struct IMAPReply {
    var records: [IMAPRecord]
    var text: String { records.map(\.line).joined(separator:"\n") }
}
final class IMAPClient {
    private let socket: MailWire
    private var serial = 0
    private(set) var capabilities = Set<String>()
    private let account: MailAccount
    init(account: MailAccount, wire:MailWire?=nil) throws {
        self.account=account
        socket=try wire ?? TLSSocket(host:account.imapHost,port:account.imapPort,implicitTLS:!account.imapStartTLS)
        let greeting=try socket.readLine()
        guard greeting.uppercased().hasPrefix("* OK") || greeting.uppercased().hasPrefix("* PREAUTH") else { throw MailFailure.message("IMAP 服务器拒绝连接") }
        if account.imapStartTLS { _=try command("STARTTLS"); try socket.startTLS() }
        if !greeting.uppercased().hasPrefix("* PREAUTH") {
            if account.oauth2 {
                let auth=Data("user=\(account.imapLogin)\u{1}auth=Bearer \(account.credential)\u{1}\u{1}".utf8).base64EncodedString()
                _=try command("AUTHENTICATE XOAUTH2",continuation:Data(auth.utf8))
            } else { _=try command("LOGIN \(Self.quote(account.imapLogin)) \(Self.quote(account.credential))",authentication:true) }
        }
        let cap=try command("CAPABILITY").text.uppercased()
        capabilities=Set(cap.split(whereSeparator:{$0.isWhitespace}).map(String.init))
        if capabilities.contains("ID") { _=try command("ID (\"name\" \"Jianxin\" \"version\" \"1.0.0\")") }
    }
    deinit { socket.close() }
    static func quote(_ text:String) -> String { "\"" + text.replacingOccurrences(of:"\\",with:"\\\\").replacingOccurrences(of:"\"",with:"\\\"").replacingOccurrences(of:"\r",with:"").replacingOccurrences(of:"\n",with:"") + "\"" }
    func command(_ text:String, continuation:Data?=nil, authentication:Bool=false) throws -> IMAPReply {
        serial += 1; let tag=String(format:"J%04d",serial); socket.resetDeadline(); try socket.line(tag+" "+text)
        var records:[IMAPRecord]=[]; var supplied=false;var total=0;var continuations=0
        while true {
            let line=try socket.readLine()
            if line.hasPrefix(tag+" ") {
                guard line.uppercased().hasPrefix(tag+" OK") else { throw MailFailure.message(authentication || text.hasPrefix("AUTHENTICATE") ? "邮箱登录失败，检查授权码或 OAuth 令牌" : "邮箱不支持或拒绝该操作，请刷新后重试") }
                return IMAPReply(records:records)
            }
            if line.hasPrefix("+") {
                continuations += 1;guard continuations<=3 else{throw MailFailure.message("邮箱认证响应异常")}
                guard let bytes=continuation,!supplied else { try socket.line(""); continue }
                try socket.write(bytes); try socket.line(""); supplied=true; continue
            }
            var literal:Data?
            if let match=line.range(of:"\\{[0-9]+\\+?\\}$",options:.regularExpression) {
                let count=Int(line[match].filter(\.isNumber)) ?? -1;guard count>=0,count<=TLSSocket.maxMessage-total else{throw MailFailure.message("邮件响应超过 25 MB")};total += count;guard total<=TLSSocket.maxMessage else{throw MailFailure.message("邮件响应超过 25 MB")};literal=try socket.readBytes(count)
            }
            if line.hasPrefix("* ") || records.isEmpty { records.append(IMAPRecord(line:line,literals:literal.map{[$0]} ?? [])) }
            else { records[records.count-1].line += " "+line; if let literal { records[records.count-1].literals.append(literal) } }
        }
    }
    func folders() throws -> [MailFolder] {
        let reply=try command("LIST \"\" \"*\"")
        return reply.records.compactMap { record in
            guard record.line.uppercased().hasPrefix("* LIST "),let close=record.line.firstIndex(of:")") else{return nil}
            let flags=String(record.line[..<close]).lowercased()
            guard !flags.contains("\\noselect") else{return nil}
            let tokens=Self.tokens(String(record.line[record.line.index(after:close)...]))
            let path: String
            if let literal=record.literals.first { path=String(decoding:literal,as:UTF8.self) }
            else { guard tokens.count>=2 else{return nil}; path=tokens[1] }
            let name=Self.decodeMailbox(path)
            return MailFolder(path:path,name:name,role:account.folderOverrides?.first(where:{$0.value==path || $0.value==name})?.key ?? Self.role(name,flags:flags))
        }.sorted { ($0.role == "inbox" ? 0 : 1,$0.name)<($1.role == "inbox" ? 0 : 1,$1.name) }
    }
    func select(_ folder:String, expected:String?=nil) throws -> String {
        let reply=try command("SELECT "+Self.quote(folder))
        guard let validity=Self.capture(reply.text,"UIDVALIDITY ([0-9]+)") else{throw MailFailure.message("服务器未返回 UIDVALIDITY，无法安全读取邮件")}
        if let expected,expected != validity { throw MailFailure.message("邮箱索引已更新，请刷新列表后重新打开邮件") }
        return validity
    }
    func page(folder:String,query:String,unread:Bool,offset:Int) throws -> (messages:[MailMessage],more:Bool) {
        let validity=try select(folder)
        var filter=unread ? "UNSEEN" : "ALL"
        if !query.trimmingCharacters(in:.whitespaces).isEmpty {
            filter += " TEXT "+Self.quote(query)
        }
        let response=try command("UID SEARCH "+(query.isEmpty ? "" : "CHARSET UTF-8 ")+filter)
        let ids=response.records.filter{$0.line.hasPrefix("* SEARCH")}.flatMap{$0.line.split(separator:" ").dropFirst(2)}
            .compactMap{UInt32($0)}.sorted(by:>)
        let page=Array(ids.dropFirst(offset).prefix(30))
        if page.isEmpty{return([],false)}
        let reply=try command("UID FETCH \(page.map(String.init).joined(separator:",")) (UID FLAGS RFC822.SIZE BODY.PEEK[HEADER.FIELDS (SUBJECT FROM TO CC REPLY-TO DATE MESSAGE-ID REFERENCES IN-REPLY-TO)])")
        var messages:[MailMessage]=[]
        for record in reply.records {
            guard let uid=Self.capture(record.line,"UID ([0-9]+)"),let raw=record.literals.first else{continue}
            var message=MIME.summary(raw,account:account.id,folder:folder,validity:validity,uid:uid)
            let flags=Self.capture(record.line,"FLAGS \\(([^)]*)\\)") ?? ""
            message.isRead=flags.contains("\\Seen");message.isStarred=flags.contains("\\Flagged")
            messages.append(message)
        }
        let order=Dictionary(uniqueKeysWithValues:page.enumerated().map{(String($0.element),$0.offset)})
        return(messages.sorted{(order[$0.uid] ?? 0)<(order[$1.uid] ?? 0)},offset+page.count<ids.count)
    }
    func detail(_ summary:MailMessage) throws -> MailMessage {
        _=try select(summary.folder,expected:summary.validity)
        let reply=try command("UID FETCH \(summary.uid) (UID FLAGS RFC822.SIZE BODY.PEEK[])")
        guard let record=reply.records.first(where:{Self.capture($0.line,"UID ([0-9]+)")==summary.uid}),let raw=record.literals.first else{throw MailFailure.message("邮件已不存在，请刷新列表")}
        var message=MIME.summary(raw,account:account.id,folder:summary.folder,validity:summary.validity,uid:summary.uid)
        let parsed=try MIME.parse(raw)
        message.body=parsed.text; message.attachments=parsed.attachments
        let flags=Self.capture(record.line,"FLAGS \\(([^)]*)\\)") ?? ""
        message.isRead=flags.contains("\\Seen");message.isStarred=flags.contains("\\Flagged")
        return message
    }
    func flag(_ message:MailMessage,flag:String,value:Bool)throws {
        _=try select(message.folder,expected:message.validity)
        _=try command("UID STORE \(message.uid) \(value ? "+" : "-")FLAGS.SILENT (\(flag))")
    }
    func move(_ message:MailMessage,to folder:String)throws {
        _=try select(message.folder,expected:message.validity)
        if folder==message.folder{return}
        if capabilities.contains("MOVE"){_=try command("UID MOVE \(message.uid) "+Self.quote(folder))}
        else if capabilities.contains("UIDPLUS"){
            _=try command("UID COPY \(message.uid) "+Self.quote(folder))
            _=try command("UID STORE \(message.uid) +FLAGS.SILENT (\\Deleted)")
            _=try command("UID EXPUNGE \(message.uid)")
        }else{throw MailFailure.message("此服务器不支持安全移动邮件") }
    }
    func append(_ bytes:Data,to folder:String)throws {
        let headers=MIME.split(bytes).headers
        if let id=headers["message-id"] {
            _=try select(folder)
            let existing=try command("UID SEARCH HEADER Message-ID "+Self.quote(id))
            if existing.records.contains(where:{$0.line.hasPrefix("* SEARCH ") && $0.line.split(separator:" ").count>2}){return}
        }
        _=try command("APPEND \(Self.quote(folder)) (\\Seen) {\(bytes.count)}",continuation:bytes)
    }
    static func capture(_ text:String,_ pattern:String)->String? {
        guard let regex=try? NSRegularExpression(pattern:pattern,options:.caseInsensitive),let match=regex.firstMatch(in:text,range:NSRange(text.startIndex...,in:text)),
              let range=Range(match.range(at:1),in:text) else{return nil}
        return String(text[range])
    }
    static func tokens(_ text:String)->[String] {
        var values:[String]=[],value="",quoted=false,escape=false,started=false
        for c in text {
            if escape{value.append(c);escape=false;continue}
            if c=="\\" && quoted{escape=true;continue}
            if c=="\""{quoted.toggle();started=true;continue}
            if c.isWhitespace && !quoted{if started{values.append(value);value="";started=false}}else{value.append(c);started=true}
        }
        if started{values.append(value)};return values
    }
    static func decodeMailbox(_ path:String)->String {
        var result="",remaining=path[...]
        while let start=remaining.firstIndex(of:"&"),let end=remaining[start...].firstIndex(of:"-") {
            result += remaining[..<start];let encoded=String(remaining[remaining.index(after:start)..<end])
            if encoded.isEmpty{result += "&"}else{
                var b64=encoded.replacingOccurrences(of:",",with:"/");while b64.count%4 != 0{b64 += "="}
                result += Data(base64Encoded:b64).flatMap{String(data:$0,encoding:.utf16BigEndian)} ?? String(remaining[start...end])
            }
            remaining=remaining[remaining.index(after:end)...]
        }
        return result+remaining
    }
    static func role(_ name:String,flags:String)->String {
        let lower=name.lowercased()
        if lower=="inbox" {return "inbox"}
        for (flag,names,role) in [("\\sent",["sent","已发送","发件箱"],"sent"),("\\drafts",["draft","草稿"],"drafts"),
            ("\\trash",["trash","deleted","已删除","垃圾箱"],"trash"),("\\junk",["junk","spam","垃圾邮件"],"spam"),("\\archive",["archive","归档"],"archive")] {
            if flags.contains(flag)||names.contains(where:lower.contains){return role}
        }
        return "custom"
    }
}
