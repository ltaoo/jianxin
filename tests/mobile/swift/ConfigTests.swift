import Foundation

enum ConfigTests {
    static func check(_ value:Bool,_ message:String){precondition(value,message)}
    static func rejects(_ operation:()throws->Void){do{try operation();fatalError("invalid config accepted")}catch{}}
    static func run()throws {
        let fixture=try JSONSerialization.jsonObject(with:Data(contentsOf:URL(fileURLWithPath:CommandLine.arguments[1]))) as! [String:Any]
        let encrypted=try JSONSerialization.data(withJSONObject:fixture["envelope"]!)
        let password=fixture["password"] as! String,plain=Data((fixture["plaintext"] as! String).utf8)
        check(try ConfigEncryption.decrypt(encrypted,password:password,method:.legacyJSON)==plain,"Node/Swift Unicode crypto interoperability")
        rejects{_=try ConfigEncryption.decrypt(encrypted,password:"wrong",method:.legacyJSON)}
        var envelope=fixture["envelope"] as! [String:Any]
        envelope["ciphertext"]=Data("changed".utf8).base64EncodedString()
        rejects{_=try ConfigEncryption.decrypt(JSONSerialization.data(withJSONObject:envelope),password:password,method:.legacyJSON)}
        envelope=fixture["envelope"] as! [String:Any];envelope["iterations"]=600001
        rejects{_=try ConfigEncryption.decrypt(JSONSerialization.data(withJSONObject:envelope),password:password,method:.legacyJSON)}
        rejects{_=try ConfigEncryption.decrypt(Data(repeating:0,count:ConfigEncryption.limit+1),password:password,method:.legacyJSON)}
        rejects{_=try ConfigEncryption.decrypt(encrypted,password:"",method:.legacyJSON)}
        for method in [ConfigCipher.aes256,.aes128] {
            let raw=Data((fixture[method.rawValue] as! String).utf8)
            check(try ConfigEncryption.decrypt(raw,password:password,method:method)==plain,"pure ciphertext cross-platform interoperability")
            rejects{_=try ConfigEncryption.decrypt(raw,password:password,method:method == .aes256 ? .aes128 : .aes256)}
            rejects{_=try ConfigEncryption.decrypt(raw,password:"wrong",method:method)}
            let packed=Data(base64Encoded:String(decoding:raw,as:UTF8.self).trimmingCharacters(in:.whitespacesAndNewlines))!
            for offset in [0,12,packed.count-1] {
                var changed=packed;changed[offset] ^= 1
                rejects{_=try ConfigEncryption.decrypt(Data(changed.base64EncodedString().utf8),password:password,method:method)}
            }
        }
        rejects{_=try ConfigEncryption.decrypt(encrypted,password:password)}
        rejects{_=try ConfigEncryption.decrypt(Data("not-base64?!".utf8),password:password)}
        let parsed=try JianxinConfig.parse(plain)
        check(parsed.accounts.count==2 && parsed.warnings.count==1,"parse account configuration without desktop general secrets")
        let qq=parsed.accounts[0],custom=parsed.accounts[1]
        check(qq.imapHost=="imap.qq.com" && qq.smtpPort==465 && qq.credential=="sample-authorization-code","domain presets")
        check(custom.imapStartTLS && custom.smtpStartTLS && custom.imapLogin=="imap-login" && custom.smtpLogin=="smtp-login" && custom.folderOverrides?["sent"]=="Sent Items","explicit endpoints and distinct protocol usernames")
        var updated=qq;updated.credential="new-password"
        let other=MailAccount(email:"other@example.com",credential:"saved",imapHost:"imap.example.com",smtpHost:"smtp.example.com")
        let merged=MailAccount.merge([updated],into:[qq,other])
        check(merged.count==2 && merged[0].id==qq.id && merged[0].credential=="new-password" && merged[1]==other,"merge preserves stable cache/draft identifiers and unrelated accounts")
        let old=Data("{\"id\":\"old\",\"email\":\"old@qq.com\",\"displayName\":\"\",\"username\":\"\",\"credential\":\"saved\",\"oauth2\":false,\"imapHost\":\"imap.qq.com\",\"imapPort\":993,\"imapStartTLS\":false,\"smtpHost\":\"smtp.qq.com\",\"smtpPort\":465,\"smtpStartTLS\":false}".utf8)
        check(try JSONDecoder().decode(MailAccount.self,from:old).imapLogin=="old@qq.com","backward compatibility for existing Keychain account records")
        var root=try JSONSerialization.jsonObject(with:plain) as! [String:Any]
        var rows=root["accounts"] as! [[String:Any]]
        rows[0]["auth"]=["type":"app_password","password":["command":"never execute this"]];root["accounts"]=rows
        let refs=try JianxinConfig.parse(JSONSerialization.data(withJSONObject:root))
        check(refs.accounts.count==1 && refs.warnings.contains{$0.contains("凭据引用")},"desktop credential reference rejected")
        rows=root["accounts"] as! [[String:Any]];rows[1]["imap"]=["host":"imap.example.com","port":143,"tls":"none"];root["accounts"]=rows
        check(try JianxinConfig.parse(JSONSerialization.data(withJSONObject:root)).accounts.isEmpty,"plaintext mail connection rejected")
        let duplicate=try JianxinConfig.parse(Data("{\"accounts\":[{\"email\":\"a@qq.com\",\"auth\":{\"type\":\"password\",\"password\":\"sample\"}},{\"email\":\"a@qq.com\",\"auth\":{\"type\":\"password\",\"password\":\"another\"}}]}".utf8))
        check(duplicate.accounts.count==1 && duplicate.warnings.count==1,"duplicate imported identity rejected")
        for url in ["http://example.com/a","https://user:password@example.com/a","https://example.com/a#fragment","file:///tmp/config"]{rejects{_=try ConfigRedirects.url(url)}}
        check(try ConfigRedirects.url("https://cdn.example.com/config.enc.json?version=2").host=="cdn.example.com","CDN query URL supported")
        print("Config import: cross-platform crypto, password/tamper rejection, schema/presets, usernames, protected merge, old credentials, references and HTTPS passed")
    }
}
