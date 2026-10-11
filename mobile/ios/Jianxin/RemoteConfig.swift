import Foundation
import CoreFoundation

struct ConfigPreview {
    var accounts:[MailAccount]
    var warnings:[String]
}
enum JianxinConfig {
    static func parse(_ data:Data)throws->ConfigPreview {
        guard let root=(try? JSONSerialization.jsonObject(with:data)) as? [String:Any],let rows=root["accounts"] as? [[String:Any]],
              !rows.isEmpty,rows.count<=100 else{throw ConfigImportError.message("配置需要包含 accounts 数组，支持 1 至 100 个账号")}
        var result=ConfigPreview(accounts:[],warnings:[]),seen=Set<String>()
        for (index,row) in rows.enumerated() {
            do{
                var account=MailAccount()
                account.email=(row["email"] as? String ?? "").trimmingCharacters(in:.whitespacesAndNewlines)
                guard MailAddress.valid(account.email),let auth=row["auth"] as? [String:Any] else{throw ConfigImportError.message("邮箱地址或 auth 格式无效")}
                let type=auth["type"] as? String ?? ""
                guard ["password","app_password","oauth2","o_auth2"].contains(type) else{throw ConfigImportError.message("不支持此认证方式")}
                account.oauth2=["oauth2","o_auth2"].contains(type)
                guard let secret=auth[account.oauth2 ? "access_token" : "password"] as? String,!secret.isEmpty else{
                    throw ConfigImportError.message("需要字符串形式的密码或访问令牌；手机不能读取桌面端凭据引用")
                }
                account.credential=secret;account.username=auth["username"] as? String ?? ""
                account.displayName=row["display_name"] as? String ?? row["id"] as? String ?? ""
                let domain=account.email.split(separator:"@").last!.lowercased()
                let preset=presets[domain] ?? ("imap."+domain,"smtp."+domain,465,false)
                account.imapHost=preset.0;account.smtpHost=preset.1;account.smtpPort=preset.2;account.smtpStartTLS=preset.3
                for kind in ["imap","smtp"] {
                    if let value=row[kind],!(value is NSNull) {
                        guard let server=value as? [String:Any],let host=server["host"] as? String,
                              let port=server["port"] as? NSNumber,CFGetTypeID(port) != CFBooleanGetTypeID(),port.doubleValue==Double(port.intValue) else{
                            throw ConfigImportError.message("服务器需要 host 和整数 port")
                        }
                        let tls=server["tls"] as? String ?? "tls"
                        guard ["tls","starttls","start_tls"].contains(tls) else{throw ConfigImportError.message("手机仅支持 TLS 或 STARTTLS，不能导入明文连接")}
                        if kind=="imap"{account.imapHost=host;account.imapPort=port.intValue;account.imapStartTLS=tls != "tls";account.imapUsername=server["username"] as? String}
                        else{account.smtpHost=host;account.smtpPort=port.intValue;account.smtpStartTLS=tls != "tls";account.smtpUsername=server["username"] as? String}
                    }
                }
                if let roles=row["folders"] as? [String:Any] {
                    account.folderOverrides=roles.compactMapValues{$0 as? String}.filter{["inbox","sent","drafts","trash","spam","archive"].contains($0.key)}
                }
                try account.validate()
                guard seen.insert(account.importIdentity).inserted else{throw ConfigImportError.message("配置中有重复邮箱和收信服务器")}
                result.accounts.append(account)
                if account.oauth2{result.warnings.append("第 \(index+1) 个账号使用已有 OAuth 令牌，到期后需更新配置；不导入桌面端刷新流程。")}
            }catch{result.warnings.append("第 \(index+1) 个账号未导入："+((error as? ConfigImportError)?.localizedDescription ?? "邮箱或服务器设置无效"))}
        }
        if root["general"] != nil{result.warnings.append("仅导入邮箱账号。桌面端 general 设置（含 API、Agent、同步策略）不应用到手机。")}
        return result
    }
    private static let presets:[String:(String,String,Int,Bool)]=[
        "gmail.com":("imap.gmail.com","smtp.gmail.com",465,false),"googlemail.com":("imap.gmail.com","smtp.gmail.com",465,false),
        "qq.com":("imap.qq.com","smtp.qq.com",465,false),"foxmail.com":("imap.qq.com","smtp.qq.com",465,false),
        "163.com":("imap.163.com","smtp.163.com",465,false),"126.com":("imap.126.com","smtp.126.com",465,false),
        "yeah.net":("imap.yeah.net","smtp.yeah.net",465,false),"aliyun.com":("imap.aliyun.com","smtp.aliyun.com",465,false),
        "sina.com":("imap.sina.com","smtp.sina.com",465,false),"139.com":("imap.139.com","smtp.139.com",465,false),
        "zoho.com":("imap.zoho.com","smtp.zoho.com",465,false),"yahoo.com":("imap.mail.yahoo.com","smtp.mail.yahoo.com",465,false),
        "fastmail.com":("imap.fastmail.com","smtp.fastmail.com",465,false),
        "icloud.com":("imap.mail.me.com","smtp.mail.me.com",587,true),"me.com":("imap.mail.me.com","smtp.mail.me.com",587,true),
        "outlook.com":("outlook.office365.com","smtp-mail.outlook.com",587,true),"hotmail.com":("outlook.office365.com","smtp-mail.outlook.com",587,true),
        "live.com":("outlook.office365.com","smtp-mail.outlook.com",587,true),"office365.com":("outlook.office365.com","smtp-mail.outlook.com",587,true)]
}

final class ConfigRedirects:NSObject,URLSessionTaskDelegate {
    private let lock=NSLock()
    private var redirects=0
    static func url(_ value:String)throws->URL {
        guard let url=URL(string:value),url.scheme?.lowercased()=="https",let host=url.host,!host.isEmpty,
              url.user==nil,url.password==nil,url.fragment==nil else{throw ConfigImportError.message("请输入完整 HTTPS URL，不要包含用户名、密码或片段")}
        return url
    }
    func urlSession(_ session:URLSession,task:URLSessionTask,willPerformHTTPRedirection response:HTTPURLResponse,newRequest request:URLRequest,completionHandler:@escaping(URLRequest?)->Void) {
        // Refuse downgrade and credential-bearing redirects before issuing any request.
        lock.lock();redirects += 1;let count=redirects;lock.unlock()
        completionHandler((try? Self.url(request.url?.absoluteString ?? "")) == nil || count>5 ? nil : request)
    }
}
enum RemoteConfig {
    static func fetch(_ value:String,password:String,method:ConfigCipher = .aes256,configuration supplied:URLSessionConfiguration?=nil)async throws->ConfigPreview {
        let url=try ConfigRedirects.url(value)
        let configuration=supplied ?? URLSessionConfiguration.ephemeral
        configuration.urlCache=nil;configuration.httpCookieStorage=nil;configuration.urlCredentialStorage=nil
        configuration.timeoutIntervalForRequest=30;configuration.timeoutIntervalForResource=60
        let session=URLSession(configuration:configuration,delegate:ConfigRedirects(),delegateQueue:nil)
        defer{session.invalidateAndCancel()}
        var request=URLRequest(url:url,cachePolicy:.reloadIgnoringLocalAndRemoteCacheData)
        request.setValue("no-cache",forHTTPHeaderField:"Cache-Control")
        do{
            let (stream,response)=try await session.bytes(for:request)
            guard let http=response as? HTTPURLResponse,(200...299).contains(http.statusCode) else{throw ConfigImportError.message("下载失败，请检查 URL 和 CDN 文件权限")}
            guard response.expectedContentLength<=ConfigEncryption.limit else{throw ConfigImportError.message("加密配置不能超过 2 MB")}
            var data=Data()
            for try await byte in stream{try Task.checkCancellation();guard data.count<ConfigEncryption.limit else{throw ConfigImportError.message("加密配置不能超过 2 MB")};data.append(byte)}
            let downloaded=data
            return try await Task.detached(priority:.userInitiated){try JianxinConfig.parse(ConfigEncryption.decrypt(downloaded,password:password,method:method))}.value
        }catch let error as ConfigImportError{throw error}
        catch is CancellationError{throw CancellationError()}
        catch{throw ConfigImportError.message("无法下载配置，请检查网络、URL 和 HTTPS 证书后重试")}
    }
}
