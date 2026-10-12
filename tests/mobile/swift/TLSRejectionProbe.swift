import Foundation
@main struct TLSRejectionProbe {
    static func main()async {
        let port=Int(CommandLine.arguments[1])!
        if CommandLine.arguments[2]=="https" {
            do{_=try await RemoteConfig.fetch("https://localhost:\(port)/config.enc.json",password:"must-never-leave-device");print("ERROR: untrusted HTTPS certificate accepted");exit(1)}
            catch{print("Untrusted HTTPS certificate rejected");exit(0)}
        }
        let startTLS=CommandLine.arguments[2]=="starttls"
        let account=MailAccount(email:"test@example.com",credential:"must-never-be-plaintext",imapHost:"localhost",imapPort:port,imapStartTLS:startTLS,smtpHost:"localhost")
        do{_=try IMAPClient(account:account);print("ERROR: untrusted certificate accepted");exit(1)}
        catch{print("Untrusted TLS certificate rejected");exit(0)}
    }
}
