import Foundation
import CryptoKit
import CommonCrypto

enum ConfigImportError: LocalizedError {
    case message(String)
    var errorDescription: String? { if case .message(let value)=self{return value};return nil }
}

enum ConfigCipher: String, CaseIterable, Identifiable {
    case aes256 = "aes-256-gcm", aes128 = "aes-128-gcm", legacyJSON = "legacy-json"
    var id:String{rawValue}
    var title:String{switch self{case .aes256:return "AES-256-GCM";case .aes128:return "AES-128-GCM";case .legacyJSON:return "旧版 JSON（兼容）"}}
    var keySize:Int{self == .aes128 ? 16 : 32}
}

/// Raw authenticated ciphertext; the caller selects its algorithm.
enum ConfigEncryption {
    static let limit=2*1024*1024
    static let aad=Data("jianxin-config:v1:pbkdf2-sha256:600000:aes-256-gcm".utf8)
    private struct Envelope: Decodable {
        let format:String, version:Int, kdf:String, iterations:Int, cipher:String
        let salt:String, nonce:String, ciphertext:String, tag:String
    }
    static func decrypt(_ data:Data,password:String,method:ConfigCipher = .aes256)throws->Data {
        guard data.count<=limit else{throw ConfigImportError.message("加密配置不能超过 2 MB")}
        guard !password.isEmpty,password.utf8.count<=1024 else{throw ConfigImportError.message("请输入解密密码（最多 1024 字节）")}
        if method == .legacyJSON{return try decryptLegacy(data,password:password)}
        guard let text=String(data:data,encoding:.utf8) else{throw ConfigImportError.message("密文必须是完整的 Base64 内容")}
        let encoded=text.replacingOccurrences(of:"[ \t\r\n]",with:"",options:.regularExpression)
        guard let packed=Data(base64Encoded:encoded),packed.count>28,packed.base64EncodedString()==encoded else{
            throw ConfigImportError.message("密文必须是完整的 Base64 内容；旧文件请选择旧版 JSON")
        }
        var key=try derive(password:password,salt:Data(("jianxin-config:password-key:v2:"+method.rawValue).utf8),size:method.keySize)
        defer{key.resetBytes(in:0..<key.count)}
        do{
            let box=try AES.GCM.SealedBox(combined:packed)
            let context=Data(("jianxin-config:v2:pbkdf2-sha256:600000:"+method.rawValue).utf8)
            return try AES.GCM.open(box,using:SymmetricKey(data:key),authenticating:context)
        }catch{throw ConfigImportError.message("解密失败：密码、加密方式不匹配或文件已被修改")}
    }
    private static func derive(password:String,salt:Data,size:Int)throws->Data {
        var key=Data(count:size)
        let passwordBytes=Data(password.utf8)
        let status=key.withUnsafeMutableBytes { output in passwordBytes.withUnsafeBytes { secret in salt.withUnsafeBytes { input in
            CCKeyDerivationPBKDF(CCPBKDFAlgorithm(kCCPBKDF2),secret.baseAddress?.assumingMemoryBound(to:CChar.self),passwordBytes.count,
                input.baseAddress?.assumingMemoryBound(to:UInt8.self),salt.count,CCPseudoRandomAlgorithm(kCCPRFHmacAlgSHA256),600000,
                output.baseAddress?.assumingMemoryBound(to:UInt8.self),size)
        }}}
        guard status==kCCSuccess else{throw ConfigImportError.message("无法派生解密密钥，请重试")}
        return key
    }
    private static func decryptLegacy(_ data:Data,password:String)throws->Data {
        guard data.count<=limit else{throw ConfigImportError.message("加密配置不能超过 2 MB")}
        guard !password.isEmpty,password.utf8.count<=1024 else{throw ConfigImportError.message("请输入解密密码（最多 1024 字节）")}
        guard let envelope=try? JSONDecoder().decode(Envelope.self,from:data),envelope.format=="jianxin-config",
              envelope.version==1,envelope.kdf=="pbkdf2-sha256",envelope.iterations==600000,envelope.cipher=="aes-256-gcm",
              let salt=Data(base64Encoded:envelope.salt),salt.count==16,
              let nonce=Data(base64Encoded:envelope.nonce),nonce.count==12,
              let ciphertext=Data(base64Encoded:envelope.ciphertext),!ciphertext.isEmpty,
              let tag=Data(base64Encoded:envelope.tag),tag.count==16 else{
            throw ConfigImportError.message("加密格式不支持或文件损坏，请使用配套加密工具")
        }
        var key=try derive(password:password,salt:salt,size:32)
        defer{key.resetBytes(in:0..<key.count)}
        do{
            let box=try AES.GCM.SealedBox(nonce:AES.GCM.Nonce(data:nonce),ciphertext:ciphertext,tag:tag)
            return try AES.GCM.open(box,using:SymmetricKey(data:key),authenticating:aad)
        }catch{throw ConfigImportError.message("解密失败：密码错误或文件已被修改")}
    }
}
