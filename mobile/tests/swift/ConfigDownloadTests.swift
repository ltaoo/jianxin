import Foundation

private final class ConfigProtocol:URLProtocol {
    static var body=Data(),code=200,length:Int?
    static var seen:[URLRequest]=[]
    override class func canInit(with request:URLRequest)->Bool{true}
    override class func canonicalRequest(for request:URLRequest)->URLRequest{request}
    override func startLoading(){
        Self.seen.append(request)
        let headers=Self.length.map{["Content-Length":String($0)]} ?? [:]
        client?.urlProtocol(self,didReceive:HTTPURLResponse(url:request.url!,statusCode:Self.code,httpVersion:"HTTP/1.1",headerFields:headers)!,cacheStoragePolicy:.notAllowed)
        client?.urlProtocol(self,didLoad:Self.body);client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading(){}
}
enum ConfigDownloadTests {
    static func run()async throws{
        let fixture=try JSONSerialization.jsonObject(with:Data(contentsOf:URL(fileURLWithPath:CommandLine.arguments[1]))) as! [String:Any]
        ConfigProtocol.body=Data((fixture["aes-256-gcm"] as! String).utf8)
        let password=fixture["password"] as! String
        let configuration=URLSessionConfiguration.ephemeral;configuration.protocolClasses=[ConfigProtocol.self]
        let preview=try await RemoteConfig.fetch("https://cdn.example.com/config.enc.json",password:password,configuration:configuration)
        precondition(preview.accounts.count==2 && ConfigProtocol.seen.last?.value(forHTTPHeaderField:"Cache-Control")=="no-cache","download-to-decrypt integration")
        ConfigProtocol.code=403
        do{_=try await RemoteConfig.fetch("https://cdn.example.com/config.enc.json",password:password,configuration:configuration);fatalError("HTTP error accepted")}catch let e as ConfigImportError{precondition(e.localizedDescription.contains("下载失败"))}
        ConfigProtocol.code=200;ConfigProtocol.length=ConfigEncryption.limit+1
        do{_=try await RemoteConfig.fetch("https://cdn.example.com/config.enc.json",password:password,configuration:configuration);fatalError("oversized response accepted")}catch let e as ConfigImportError{precondition(e.localizedDescription.contains("2 MB"))}
        ConfigProtocol.length=nil;ConfigProtocol.body=Data(repeating:0,count:ConfigEncryption.limit+1)
        do{_=try await RemoteConfig.fetch("https://cdn.example.com/config.enc.json",password:password,configuration:configuration);fatalError("oversized stream accepted")}catch let e as ConfigImportError{precondition(e.localizedDescription.contains("2 MB"))}
        let redirects=ConfigRedirects(),session=URLSession(configuration:.ephemeral),task=session.dataTask(with:URL(string:"https://cdn.example.com/a")!)
        let response=HTTPURLResponse(url:URL(string:"https://cdn.example.com/a")!,statusCode:302,httpVersion:nil,headerFields:[:])!
        var received:URLRequest?
        redirects.urlSession(session,task:task,willPerformHTTPRedirection:response,newRequest:URLRequest(url:URL(string:"http://example.com/config")!)){received=$0}
        precondition(received==nil,"HTTP downgrade redirected")
        for _ in 0..<4{redirects.urlSession(session,task:task,willPerformHTTPRedirection:response,newRequest:URLRequest(url:URL(string:"https://other.example.com/config")!)){received=$0};precondition(received != nil)}
        redirects.urlSession(session,task:task,willPerformHTTPRedirection:response,newRequest:URLRequest(url:URL(string:"https://other.example.com/config")!)){received=$0}
        precondition(received==nil,"too many redirects accepted");session.invalidateAndCancel()
        print("Remote download: HTTPS download/decrypt, cache bypass, HTTP errors, declared/streamed size limits and redirect downgrade/loop rejection passed")
    }
}
