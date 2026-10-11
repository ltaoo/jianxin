import Foundation
import CFNetwork

/// A bounded blocking stream used exclusively on a background worker. System trust validation stays enabled.
protocol MailWire: AnyObject {
    func resetDeadline()
    func startTLS() throws
    func write(_ data:Data) throws
    func line(_ text:String) throws
    func readLine() throws -> String
    func readBytes(_ count:Int) throws -> Data
    func close()
}
final class TLSSocket: MailWire {
    private let input: CFReadStream
    private let output: CFWriteStream
    private let host: String
    private var buffer = Data()
    private var deadline = Date()
    static let maxMessage = 25 * 1024 * 1024
    init(host: String, port: Int, implicitTLS: Bool) throws {
        self.host=host
        var read: Unmanaged<CFReadStream>?, write: Unmanaged<CFWriteStream>?
        CFStreamCreatePairWithSocketToHost(nil,host as CFString,UInt32(port),&read,&write)
        guard let read=read?.takeRetainedValue(),let write=write?.takeRetainedValue() else { throw MailFailure.message("无法创建邮箱连接") }
        input=read; output=write
        CFReadStreamScheduleWithRunLoop(input,CFRunLoopGetCurrent(),CFRunLoopMode.defaultMode)
        CFWriteStreamScheduleWithRunLoop(output,CFRunLoopGetCurrent(),CFRunLoopMode.defaultMode)
        if implicitTLS { try startTLS() }
        guard CFReadStreamOpen(input),CFWriteStreamOpen(output) else { throw MailFailure.message("无法连接邮件服务器") }
        resetDeadline()
    }
    deinit { close() }
    func close() {
        CFReadStreamClose(input); CFWriteStreamClose(output)
        CFReadStreamUnscheduleFromRunLoop(input,CFRunLoopGetCurrent(),CFRunLoopMode.defaultMode)
        CFWriteStreamUnscheduleFromRunLoop(output,CFRunLoopGetCurrent(),CFRunLoopMode.defaultMode)
    }
    func resetDeadline() { deadline=Date().addingTimeInterval(60) }
    func startTLS() throws {
        guard buffer.isEmpty else { throw MailFailure.message("TLS 升级响应异常") }
        let settings: [String:Any] = [kCFStreamSSLLevel as String:kCFStreamSocketSecurityLevelNegotiatedSSL,
            kCFStreamSSLPeerName as String:host,kCFStreamSSLValidatesCertificateChain as String:true]
        guard CFReadStreamSetProperty(input,CFStreamPropertyKey(rawValue:kCFStreamPropertySSLSettings),settings as CFDictionary),
              CFWriteStreamSetProperty(output,CFStreamPropertyKey(rawValue:kCFStreamPropertySSLSettings),settings as CFDictionary) else { throw MailFailure.message("邮件服务器无法建立安全连接") }
    }
    private func wait(_ ready: () -> Bool) throws {
        while !ready() {
            if CFReadStreamGetStatus(input) == .error || CFWriteStreamGetStatus(output) == .error {
                throw MailFailure.message("安全连接失败，检查服务器地址与证书")
            }
            if CFReadStreamGetStatus(input) == .atEnd || CFReadStreamGetStatus(input) == .closed || Date() > deadline {
                throw MailFailure.message("邮箱连接超时或已断开，请重试")
            }
            CFRunLoopRunInMode(CFRunLoopMode.defaultMode,0.01,true)
        }
    }
    func write(_ data: Data) throws {
        var offset=0
        while offset<data.count {
            try wait { CFWriteStreamCanAcceptBytes(output) }
            let n=data.withUnsafeBytes { bytes in CFWriteStreamWrite(output,bytes.baseAddress!.assumingMemoryBound(to:UInt8.self).advanced(by:offset),data.count-offset) }
            guard n>0 else { throw MailFailure.message("无法写入邮箱连接") }; offset += n
        }
    }
    func line(_ text:String) throws { try write(Data((text+"\r\n").utf8)) }
    private func fill() throws {
        try wait { CFReadStreamHasBytesAvailable(input) }
        var bytes=[UInt8](repeating:0,count:8192)
        let count=CFReadStreamRead(input,&bytes,bytes.count)
        guard count>0 else { throw MailFailure.message("邮箱连接已断开") }
        buffer.append(contentsOf:bytes.prefix(count))
    }
    func readLine() throws -> String {
        while true {
            if let range=buffer.range(of:Data([13,10])) {
                let data=buffer[..<range.lowerBound]; buffer.removeSubrange(..<range.upperBound)
                return String(decoding:data,as:UTF8.self)
            }
            guard buffer.count < 256*1024 else { throw MailFailure.message("邮件服务器响应过长") }
            try fill()
        }
    }
    func readBytes(_ count:Int) throws -> Data {
        guard count>=0,count<=Self.maxMessage else { throw MailFailure.message("邮件超过 25 MB，使用桌面客户端查看") }
        var result=Data(); result.reserveCapacity(count)
        while result.count<count {
            if buffer.isEmpty { try fill() }
            let n=min(count-result.count,buffer.count); result.append(buffer.prefix(n)); buffer.removeFirst(n)
        }
        return result
    }
}

func mailWork<T>(_ work: @escaping () throws -> T) async throws -> T {
    try await withCheckedThrowingContinuation { continuation in
        DispatchQueue.global(qos:.userInitiated).async {
            do { continuation.resume(returning:try work()) } catch { continuation.resume(throwing:error) }
        }
    }
}
