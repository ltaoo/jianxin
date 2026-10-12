import Foundation

final class ScriptWire: MailWire {
    var lines:[String]
    var literals:[Data]
    var sent:[String]=[]
    var closed=false
    var upgraded=false
    init(_ lines:[String],literals:[Data]=[]){self.lines=lines;self.literals=literals}
    func resetDeadline(){}
    func startTLS()throws{upgraded=true}
    func write(_ data:Data)throws{sent.append(String(decoding:data,as:UTF8.self))}
    func line(_ text:String)throws{sent.append(text+"\r\n")}
    func readLine()throws->String{guard !lines.isEmpty else{throw MailFailure.message("mock EOF")};return lines.removeFirst()}
    func readBytes(_ count:Int)throws->Data{let bytes=literals.removeFirst();precondition(bytes.count==count);return bytes}
    func close(){closed=true}
}
@main struct ProtocolTests {
    static func check(_ value:Bool,_ message:String){if !value{fatalError(message)}}
    static func main()async throws {
        try ConfigTests.run()
        try await ConfigDownloadTests.run()
        let account=MailAccount(email:"a@example.com",credential:"app-pass",imapHost:"imap.example.com",smtpHost:"smtp.example.com")
        check(MIME.decodeHeader("=?UTF-8?B?5L2g5aW9?=")=="你好","unicode subject")
        check(MIME.decodeHeader(MIME.encodedHeader(String(repeating:"长主题",count:40)))==String(repeating:"长主题",count:40),"folded unicode subject")
        let addresses=MailAddress.parseList("\"Doe, John\" <john@example.com>, Alice <a@example.com>")
        check(addresses.count==2 && addresses[0].name=="Doe, John","quoted comma")
        check(IMAPClient.decodeMailbox("&XfJT0ZAB-")=="已发送","modified UTF-7")
        let html="<p>你好 &amp; &#x4f60;</p><script>bad()</script><img src='https://tracker.test'><style>bad</style>"
        check(MIME.htmlText(html)=="你好 & 你","native HTML plain text and no scripts")
        let binary=Data([0,1,10,13,255])
        let draft=ComposeDraft(accountId:account.id,to:"bob@example.com",cc:"cc@example.com",bcc:"secret@example.com",subject:"主题",text:"你好\n正文",inReplyTo:"<parent@example.com>",references:["<parent@example.com>"],attachments:[MailAttachment(id:"1",filename:"资料.bin",mimeType:"application/octet-stream",size:binary.count,data:binary)])
        let bytes=try MIME.compose(draft,account:account),parsed=try MIME.parse(bytes)
        let header=MIME.split(bytes).headers
        check(header["bcc"]==nil,"bcc must not leak in headers")
        check(header["in-reply-to"]=="<parent@example.com>","thread headers")
        check(parsed.text==draft.text && parsed.attachments.count==1 && parsed.attachments[0].data==binary && parsed.attachments[0].filename=="资料.bin","multipart binary roundtrip")
        check(String(decoding:SMTPClient.dotStuff(Data(".one\r\n..two\r\n".utf8)),as:UTF8.self)=="..one\r\n...two\r\n","dot stuffing")
        let raw=Data("Subject: =?UTF-8?B?5L2g5aW9?=\r\nFrom: Alice <alice@example.com>\r\n\r\n".utf8)
        let wire=ScriptWire(["* OK hello","J0001 OK login","* CAPABILITY IMAP4rev1 MOVE","J0002 OK cap","* OK [UIDVALIDITY 7] valid","J0003 OK select","* SEARCH 1 2","J0004 OK search","* 2 FETCH (BODY[HEADER.FIELDS (SUBJECT)] {\(raw.count)}", " UID 2 FLAGS (\\Seen \\Flagged))", "J0005 OK fetch"],literals:[raw])
        let imap=try IMAPClient(account:account,wire:wire)
        let page=try imap.page(folder:"INBOX",query:"",unread:false,offset:0)
        check(page.messages.count==1 && page.messages[0].uid=="2" && page.messages[0].isRead && page.messages[0].isStarred,"literal followed by UID/FLAGS")
        let reset=ScriptWire(["* OK hello","J0001 OK login","* CAPABILITY IMAP4rev1","J0002 OK cap","* OK [UIDVALIDITY 8] valid","J0003 OK select"])
        let restarted=try IMAPClient(account:account,wire:reset)
        do{try restarted.flag(page.messages[0],flag:"\\Seen",value:false);fatalError("stale UID accepted")}catch{}
        check(!reset.sent.contains(where:{$0.contains("UID STORE")}),"stale UID must never be mutated")
        let smtp=ScriptWire(["220 hello","250-host","250 AUTH PLAIN","235 ok","250 from","250 to","250 cc","250 bcc","354 go","250 accepted"])
        _=try SMTPClient.send(draft,account:account,connect:{smtp})
        check(smtp.sent.contains("RCPT TO:<secret@example.com>\r\n") && smtp.closed,"bcc SMTP envelope and close")
        let lost=ScriptWire(["220 hello","250 AUTH PLAIN","235 ok","250 from","250 to","250 cc","250 bcc","354 go"])
        do{_=try SMTPClient.send(draft,account:account,connect:{lost});fatalError("lost SMTP acceptance was success")}catch MailFailure.uncertain{}
        let rejected=ScriptWire(["220 hello","250 AUTH PLAIN","535 failed"])
        do{_=try SMTPClient.send(draft,account:account,connect:{rejected});fatalError("auth rejection accepted")}catch MailFailure.message{}
        var injected=draft;injected.subject="bad\r\nBcc: evil@example.com"
        do{_=try MIME.compose(injected,account:account);fatalError("header injection accepted")}catch{}
        let segmented=Data("Content-Type: application/octet-stream\r\nContent-Disposition: attachment; filename*0*=UTF-8''%E8%B5%84; filename*1*=%E6%96%99.bin\r\n\r\nabc".utf8)
        check((try MIME.parse(segmented)).attachments[0].filename=="资料.bin","RFC2231 segmented filename")
        let fallback=ScriptWire(["* OK hello","J0001 OK login","* CAPABILITY IMAP4rev1 UIDPLUS","J0002 OK cap","* OK [UIDVALIDITY 7] valid","J0003 OK select","J0004 OK copy","J0005 OK store","J0006 OK expunge"])
        try IMAPClient(account:account,wire:fallback).move(page.messages[0],to:"Trash")
        check(fallback.sent.contains("J0006 UID EXPUNGE 2\r\n") && !fallback.sent.contains("EXPUNGE\r\n"),"fallback expunges only the selected UID")
        let refused=ScriptWire(["220 hello","250 AUTH PLAIN","235 ok","250 from","250 to","250 cc","250 bcc","354 go","550 refused"])
        do{_=try SMTPClient.send(draft,account:account,connect:{refused});fatalError("explicit refusal accepted")}catch MailFailure.message{}
        print("Swift native MIME, IMAP and SMTP protocol tests passed")
    }
}
