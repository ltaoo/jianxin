import Foundation
import Security
import CryptoKit

enum CredentialStore {
    private static let query: [String: Any] = [kSecClass as String:kSecClassGenericPassword,
        kSecAttrService as String:"com.jianxin.mail.accounts", kSecAttrAccount as String:"mailboxes"]
    static func load() -> [MailAccount] {
        var request=query; request[kSecReturnData as String]=true; request[kSecMatchLimit as String]=kSecMatchLimitOne
        var item: CFTypeRef?
        guard SecItemCopyMatching(request as CFDictionary,&item)==errSecSuccess, let data=item as? Data,
              let accounts=try? JSONDecoder().decode([MailAccount].self,from:data) else { return [] }
        return accounts
    }
    static func save(_ accounts: [MailAccount]) throws {
        let attributes: [String:Any] = [kSecValueData as String:try JSONEncoder().encode(accounts),
            kSecAttrAccessible as String:kSecAttrAccessibleWhenUnlockedThisDeviceOnly]
        let status=SecItemUpdate(query as CFDictionary,attributes as CFDictionary)
        if status==errSecItemNotFound {
            var entry=query; attributes.forEach { entry[$0]=$1 }
            guard SecItemAdd(entry as CFDictionary,nil)==errSecSuccess else { throw MailFailure.message("无法安全保存邮箱凭据") }
        } else if status != errSecSuccess { throw MailFailure.message("无法安全保存邮箱凭据") }
    }
}
final class LocalStore {
    private let directory: URL
    init() {
        directory=FileManager.default.urls(for:.applicationSupportDirectory,in:.userDomainMask)[0].appendingPathComponent("Mail",isDirectory:true)
        try? FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
        var root=directory; var values=URLResourceValues(); values.isExcludedFromBackup=true; try? root.setResourceValues(values)
    }
    private func file(_ key: String) -> URL {
        let hash=SHA256.hash(data:Data(key.utf8)).map { String(format:"%02x",$0) }.joined()
        return directory.appendingPathComponent(hash+".json")
    }
    func read<T:Decodable>(_ key:String,as:T.Type)->T? { guard let data=try? Data(contentsOf:file(key)) else{return nil}; return try? JSONDecoder().decode(T.self,from:data) }
    func write<T:Encodable>(_ key:String,_ value:T)throws { try JSONEncoder().encode(value).write(to:file(key),options:[.atomic,.completeFileProtection]) }
    func remove(_ key:String) { try? FileManager.default.removeItem(at:file(key)) }
}
