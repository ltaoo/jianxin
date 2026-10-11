import Foundation
import CoreFoundation

/// MIME is decoded locally into native text. No HTML renderer or remote image requests.
enum MIME {
    struct Content { var text:String; var attachments:[MailAttachment] }
    static func split(_ data:Data)->(headers:[String:String],body:Data) {
        let separator=data.range(of:Data("\r\n\r\n".utf8)) ?? data.range(of:Data("\n\n".utf8))
        let header=separator.map{Data(data[..<$0.lowerBound])} ?? data
        let body=separator.map{Data(data[$0.upperBound...])} ?? Data()
        let raw=String(data:header,encoding:.utf8) ?? (String(data:header,encoding:.isoLatin1) ?? String(decoding:header,as:UTF8.self))
        let unfolded=raw.replacingOccurrences(of:"\r\n",with:"\n").replacingOccurrences(of:"\n[ \\t]+",with:" ",options:.regularExpression)
        var headers:[String:String]=[:]
        for line in unfolded.split(separator:"\n") {
            guard let colon=line.firstIndex(of:":") else{continue}
            let key=line[..<colon].lowercased(),value=line[line.index(after:colon)...].trimmingCharacters(in:.whitespaces)
            if let old=headers[key]{headers[key]=old+", "+value}else{headers[key]=value}
        }
        return(headers,body)
    }
    static func decode(_ data:Data,charset:String)->String {
        let cf=CFStringConvertIANACharSetNameToEncoding(charset as CFString)
        if cf != kCFStringEncodingInvalidId,let value=String(data:data,encoding:String.Encoding(rawValue:CFStringConvertEncodingToNSStringEncoding(cf))){return value}
        return String(data:data,encoding:.utf8) ?? (String(data:data,encoding:.isoLatin1) ?? String(decoding:data,as:UTF8.self))
    }
    static func quotedPrintable(_ bytes:Data,header:Bool=false)->Data {
        let raw=Array(bytes);var out:[UInt8]=[],i=0
        while i<raw.count {
            if header && raw[i]==95 {out.append(32);i += 1;continue}
            if raw[i]==61 {
                if i+1<raw.count,raw[i+1]==10{i += 2;continue}
                if i+2<raw.count,raw[i+1]==13,raw[i+2]==10{i += 3;continue}
                if i+2<raw.count,let value=UInt8(String(bytes:raw[(i+1)...(i+2)],encoding:.ascii) ?? "",radix:16){out.append(value);i += 3;continue}
            }
            out.append(raw[i]);i += 1
        }
        return Data(out)
    }
    static func decodeHeader(_ input:String)->String {
        let joined=input.replacingOccurrences(of:"(\\?=)[ \\t\\r\\n]+(=\\?)",with:"$1$2",options:.regularExpression)
        guard let regex=try? NSRegularExpression(pattern:"=\\?([^?]+)\\?([bBqQ])\\?([^?]*)\\?=") else{return joined}
        var output=joined
        for match in regex.matches(in:joined,range:NSRange(joined.startIndex...,in:joined)).reversed() {
            let range=Range(match.range,in:joined)!,charset=String(joined[Range(match.range(at:1),in:joined)!]),
                mode=String(joined[Range(match.range(at:2),in:joined)!]).lowercased(),payload=String(joined[Range(match.range(at:3),in:joined)!])
            let bytes=mode=="b" ? (Data(base64Encoded:payload,options:.ignoreUnknownCharacters) ?? Data()) : quotedPrintable(Data(payload.utf8),header:true)
            output.replaceSubrange(range,with:decode(bytes,charset:charset))
        }
        return output
    }
    static func parameter(_ value:String,_ key:String)->String? {
        let pattern="(?:^|;)\\s*"+NSRegularExpression.escapedPattern(for:key)+"\\s*=\\s*(?:\"([^\"]*)\"|([^;\\s]+))"
        guard let regex=try? NSRegularExpression(pattern:pattern,options:.caseInsensitive),let match=regex.firstMatch(in:value,range:NSRange(value.startIndex...,in:value)) else{return nil}
        for n in [1,2]{if let r=Range(match.range(at:n),in:value){return String(value[r])}}
        return nil
    }
    static func filename(_ value:String)->String? {
        for key in ["filename","name"] {
            var segments="",found=false
            for i in 0..<50 {
                guard let part=parameter(value,"\(key)*\(i)*") ?? parameter(value,"\(key)*\(i)") else{break}
                segments += part;found=true
            }
            let extended=found ? segments : parameter(value,key+"*")
            if let extended {
                let parts=extended.split(separator:"'",maxSplits:2,omittingEmptySubsequences:false)
                let raw=parts.count==3 ? String(parts[2]) : extended
                let charset=parts.count==3 ? String(parts[0]) : "utf-8"
                let bytes=Array(raw.utf8);var decoded:[UInt8]=[],i=0
                while i<bytes.count {
                    if bytes[i]==37,i+2<bytes.count,let c=UInt8(String(bytes:bytes[(i+1)...(i+2)],encoding:.ascii) ?? "",radix:16){decoded.append(c);i += 3}
                    else{decoded.append(bytes[i]);i += 1}
                }
                return decode(Data(decoded),charset:charset)
            }
            if let normal=parameter(value,key){return decodeHeader(normal)}
        }
        return nil
    }
    static func summary(_ data:Data,account:String,folder:String,validity:String,uid:String)->MailMessage {
        let h=split(data).headers
        var message=MailMessage(accountId:account,folder:folder,validity:validity,uid:uid)
        message.subject=decodeHeader(h["subject"] ?? "");message.from=MailAddress.parseList(h["from"] ?? "")
        message.to=MailAddress.parseList(h["to"] ?? "");message.cc=MailAddress.parseList(h["cc"] ?? "");message.replyTo=MailAddress.parseList(h["reply-to"] ?? "")
        message.messageId=h["message-id"] ?? "";message.references=(h["references"] ?? "").split(whereSeparator:{$0.isWhitespace}).map(String.init)
        let date=(h["date"] ?? "").replacingOccurrences(of:"\\s*\\([^)]*\\)",with:"",options:.regularExpression)
        let formatter=DateFormatter();formatter.locale=Locale(identifier:"en_US_POSIX")
        for format in ["EEE, d MMM yyyy HH:mm:ss Z","d MMM yyyy HH:mm:ss Z","EEE, d MMM yyyy HH:mm Z","d MMM yyyy HH:mm Z"] {
            formatter.dateFormat=format;if let parsed=formatter.date(from:date){message.date=parsed;break}
        }
        return message
    }
    static func parse(_ raw:Data,depth:Int=0)throws -> Content {
        guard depth<16 else{throw MailFailure.message("邮件 MIME 嵌套过深")}
        let part=split(raw),h=part.headers,type=(h["content-type"] ?? "text/plain").lowercased(),disposition=h["content-disposition"] ?? ""
        if type.hasPrefix("multipart/"),let boundary=parameter(h["content-type"] ?? "","boundary") {
            let marker=Data(("--"+boundary).utf8)
            var ranges:[Range<Data.Index>]=[],start=part.body.startIndex
            while let range=part.body.range(of:marker,in:start..<part.body.endIndex) {
                let startsLine=range.lowerBound==part.body.startIndex || part.body[part.body.index(before:range.lowerBound)]==10
                let endsToken=range.upperBound==part.body.endIndex || [UInt8(13),10,32,9,45].contains(part.body[range.upperBound])
                if startsLine && endsToken{ranges.append(range)}
                start=range.upperBound
            }
            var children:[Content]=[]
            for i in 0..<max(0,ranges.count-1) {
                let begin=ranges[i].upperBound,end=ranges[i+1].lowerBound
                if part.body[begin...].starts(with:Data("--".utf8)){break}
                var child=Data(part.body[begin..<end]);while child.first==13 || child.first==10{child.removeFirst()};while child.last==13 || child.last==10{child.removeLast()}
                children.append(try parse(child,depth:depth+1))
            }
            let attachments=children.flatMap(\.attachments)
            guard attachments.count<=100 else{throw MailFailure.message("邮件附件过多")}
            // multipart/alternative: choose one version, avoiding duplicated plain/HTML content.
            let text=type.hasPrefix("multipart/alternative") ? (children.first{!$0.text.isEmpty}?.text ?? "") : children.map(\.text).filter{!$0.isEmpty}.joined(separator:"\n\n")
            return Content(text:text,attachments:attachments)
        }
        let transfer=(h["content-transfer-encoding"] ?? "").lowercased()
        let bytes:Data
        if transfer=="base64"{guard let decoded=Data(base64Encoded:part.body,options:.ignoreUnknownCharacters) else{throw MailFailure.message("邮件附件编码无效")};bytes=decoded}
        else if transfer=="quoted-printable"{bytes=quotedPrintable(part.body)}else{bytes=part.body}
        let name=filename(disposition) ?? filename(h["content-type"] ?? "")
        if disposition.lowercased().hasPrefix("attachment") || name != nil || (!type.hasPrefix("text/plain") && !type.hasPrefix("text/html")) {
            return Content(text:"",attachments:[MailAttachment(id:UUID().uuidString,filename:name ?? "attachment",mimeType:type.components(separatedBy:";")[0],size:bytes.count,data:bytes)])
        }
        let text=decode(bytes,charset:parameter(h["content-type"] ?? "","charset") ?? "utf-8")
        return Content(text:type.hasPrefix("text/html") ? htmlText(text) : text,attachments:[])
    }
    static func htmlText(_ html:String)->String {
        var text=html.replacingOccurrences(of:"<(script|style|head)\\b[^>]*>[\\s\\S]*?</\\1\\s*>",with:"",options:[.regularExpression,.caseInsensitive])
        text=text.replacingOccurrences(of:"</?(p|div|br|li|tr|h[1-6]|blockquote|pre)\\b[^>]*>",with:"\n",options:[.regularExpression,.caseInsensitive])
        text=text.replacingOccurrences(of:"<[^>]*>",with:"",options:.regularExpression)
        let entities=["amp":"&","lt":"<","gt":">","quot":"\"","apos":"'","nbsp":" ","ndash":"–","mdash":"—","hellip":"…","copy":"©"]
        if let regex=try? NSRegularExpression(pattern:"&(#x[0-9a-fA-F]+|#[0-9]+|[A-Za-z]+);") {
            for match in regex.matches(in:text,range:NSRange(text.startIndex...,in:text)).reversed() {
                guard let r=Range(match.range,in:text),let nameRange=Range(match.range(at:1),in:text) else{continue}
                let key=String(text[nameRange]);var replacement=entities[key]
                if key.hasPrefix("#x"){replacement=UInt32(key.dropFirst(2),radix:16).flatMap(UnicodeScalar.init).map(String.init)}
                else if key.hasPrefix("#"){replacement=UInt32(key.dropFirst()).flatMap(UnicodeScalar.init).map(String.init)}
                if let replacement{text.replaceSubrange(r,with:replacement)}
            }
        }
        return text.replacingOccurrences(of:"\n{3,}",with:"\n\n",options:.regularExpression).trimmingCharacters(in:.whitespacesAndNewlines)
    }
    static func encodedHeader(_ value:String)->String {
        var words:[String]=[],part="",size=0
        for character in value {
            let count=String(character).utf8.count
            if size+count>42 && !part.isEmpty{words.append(part);part="";size=0}
            part.append(character);size += count
        }
        if !part.isEmpty || words.isEmpty{words.append(part)}
        return words.map{"=?UTF-8?B?"+Data($0.utf8).base64EncodedString()+"?="}.joined(separator:"\r\n ")
    }
    static func wrap(_ value:String)->String {
        stride(from:0,to:value.count,by:76).map{offset in let start=value.index(value.startIndex,offsetBy:offset),end=value.index(start,offsetBy:min(76,value.count-offset));return String(value[start..<end])}.joined(separator:"\r\n")
    }
    static func compose(_ draft:ComposeDraft,account:MailAccount)throws -> Data {
        try draft.validate();try account.validate()
        let addresses:([MailAddress])->String={list in list.map{($0.name.isEmpty ? "" : encodedHeader($0.name)+" ")+"<"+$0.email+">"}.joined(separator:", ")}
        let domain=account.email.split(separator:"@").last.map(String.init) ?? "jianxin.local"
        let formatter=DateFormatter();formatter.locale=Locale(identifier:"en_US_POSIX");formatter.dateFormat="EEE, dd MMM yyyy HH:mm:ss Z"
        var headers=["From: \(account.displayName.isEmpty ? "" : encodedHeader(account.displayName)+" ")<\(account.email)>",
            "To: "+addresses(MailAddress.parseList(draft.to)),"Subject: "+encodedHeader(draft.subject),"Date: "+formatter.string(from:draft.date),
            "Message-ID: <\(draft.id)@\(domain)>","MIME-Version: 1.0"]
        if !draft.cc.isEmpty{headers.append("Cc: "+addresses(MailAddress.parseList(draft.cc)))}
        if !draft.inReplyTo.isEmpty{headers.append("In-Reply-To: "+draft.inReplyTo)}
        if !draft.references.isEmpty{headers.append("References: "+draft.references.joined(separator:" "))}
        let text="Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n"+wrap(Data(draft.text.utf8).base64EncodedString())+"\r\n"
        if draft.attachments.isEmpty{return Data((headers.joined(separator:"\r\n")+"\r\n"+text).utf8)}
        let boundary="jx-"+draft.id;headers.append("Content-Type: multipart/mixed; boundary=\"\(boundary)\"")
        var body="\r\n--\(boundary)\r\n"+text
        for attachment in draft.attachments {
            let filename=attachment.filename.addingPercentEncoding(withAllowedCharacters:.alphanumerics) ?? "attachment"
            let type=attachment.mimeType.contains(where:{$0.isNewline}) ? "application/octet-stream" : attachment.mimeType
            body += "--\(boundary)\r\nContent-Type: \(type)\r\nContent-Disposition: attachment; filename*=UTF-8''\(filename)\r\nContent-Transfer-Encoding: base64\r\n\r\n"+wrap((attachment.data ?? Data()).base64EncodedString())+"\r\n"
        }
        body += "--\(boundary)--\r\n"
        return Data((headers.joined(separator:"\r\n")+"\r\n"+body).utf8)
    }
}
