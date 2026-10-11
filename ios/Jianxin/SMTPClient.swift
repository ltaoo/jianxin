import Foundation

private struct SMTPRejection: Error {}

enum SMTPClient {
    static func send(_ draft:ComposeDraft,account:MailAccount,connect:(()->MailWire)?=nil)throws -> Data {
        let message=try MIME.compose(draft,account:account)
        guard message.count<=TLSSocket.maxMessage else{throw MailFailure.message("邮件超过 25 MB")}
        let socket=try connect?() ?? TLSSocket(host:account.smtpHost,port:account.smtpPort,implicitTLS:!account.smtpStartTLS)
        defer{socket.close()}
        func response(_ expected:Set<Int>,final:Bool=false)throws -> String {
            socket.resetDeadline();var lines:[String]=[]
            for _ in 0..<100 {
                let line=try socket.readLine();lines.append(line)
                guard line.count>=3,let code=Int(line.prefix(3)) else{throw MailFailure.message("SMTP 响应无效")}
                if line.count<4 || line[line.index(line.startIndex,offsetBy:3)] != "-" {
                    guard expected.contains(code) else{if final{throw SMTPRejection()};throw MailFailure.message(code==535 || code==534 ? "发信登录失败，检查授权码或 OAuth 令牌" : "服务器拒绝发送，检查收件人或邮箱权限")}
                    return lines.joined(separator:"\n")
                }
            }
            throw MailFailure.message("SMTP 响应过长")
        }
        _=try response([220]);try socket.line("EHLO jianxin.local");var ehlo=try response([250])
        if account.smtpStartTLS {
            guard ehlo.uppercased().contains("STARTTLS") else{throw MailFailure.message("SMTP 服务器不支持 STARTTLS")}
            try socket.line("STARTTLS");_=try response([220]);try socket.startTLS()
            try socket.line("EHLO jianxin.local");ehlo=try response([250])
        }
        if account.oauth2 {
            let auth=Data("user=\(account.smtpLogin)\u{1}auth=Bearer \(account.credential)\u{1}\u{1}".utf8).base64EncodedString()
            try socket.line("AUTH XOAUTH2 "+auth);_=try response([235])
        }else if ehlo.uppercased().contains("PLAIN") {
            let auth=Data("\u{0}\(account.smtpLogin)\u{0}\(account.credential)".utf8).base64EncodedString()
            try socket.line("AUTH PLAIN "+auth);let reply=try response([235,334])
            if reply.hasPrefix("334"){try socket.line(auth);_=try response([235])}
        }else{
            try socket.line("AUTH LOGIN");_=try response([334]);try socket.line(Data(account.smtpLogin.utf8).base64EncodedString());_=try response([334])
            try socket.line(Data(account.credential.utf8).base64EncodedString());_=try response([235])
        }
        try socket.line("MAIL FROM:<\(account.email)>");_=try response([250])
        for email in Set(draft.recipients.map(\.email)){try socket.line("RCPT TO:<\(email)>");_=try response([250,251])}
        try socket.line("DATA");_=try response([354])
        do {
            let wire=dotStuff(message)
            try socket.write(wire);try socket.line(".")
            _=try response([250],final:true)
        }catch is SMTPRejection{throw MailFailure.message("服务器明确拒绝此邮件，请检查邮箱限制后重试")}
        catch{throw MailFailure.uncertain}
        try? socket.line("QUIT")
        return message
    }
    static func dotStuff(_ data:Data)->Data {
        let text=String(decoding:data,as:UTF8.self)
        let escaped=text.components(separatedBy:"\r\n").map{$0.hasPrefix(".") ? "."+$0 : $0}.joined(separator:"\r\n")
        return Data((escaped.hasSuffix("\r\n") ? escaped : escaped+"\r\n").utf8)
    }
}
