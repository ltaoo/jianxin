import SwiftUI

struct ConfigImportView:View {
    @EnvironmentObject var store:MailStore
    @Environment(\.dismiss) private var dismiss
    @State private var url=""
    @State private var password=""
    @State private var method=ConfigCipher.aes256
    @State private var preview:ConfigPreview?
    @State private var selected=Set<String>()
    @State private var busy=false
    @State private var error:String?
    @State private var work:Task<Void,Never>?
    var body:some View {
        NavigationStack{
            Form{
                if let error{Section{Text(error).foregroundStyle(MailTheme.danger)}}
                if let preview {
                    Section("选择要导入的账号"){
                        if preview.accounts.isEmpty{Text("没有可导入的账号，请按下方提示调整配置后重新加密。")}
                        ForEach(preview.accounts){account in
                            Toggle(isOn:Binding(get:{selected.contains(account.id)},set:{if $0{selected.insert(account.id)}else{selected.remove(account.id)}})){
                                VStack(alignment:.leading,spacing:MailTheme.space1){
                                    Text(account.title);Text(account.email).font(.caption).foregroundStyle(MailTheme.muted)
                                    Text("IMAP：\(account.imapHost):\(account.imapPort)\nSMTP：\(account.smtpHost):\(account.smtpPort)").font(.caption).foregroundStyle(MailTheme.muted)
                                    Text(store.accounts.contains{$0.importIdentity==account.importIdentity} ? "更新已有账号" : "新增账号").font(.caption).foregroundStyle(MailTheme.primary)
                                }
                            }.frame(minHeight:MailTheme.target)
                        }
                    }
                    if !preview.warnings.isEmpty{Section("配置提示"){ForEach(Array(preview.warnings.enumerated()),id:\.offset){_,warning in Text(warning).font(.caption).foregroundStyle(MailTheme.warning)}}}
                    Section{Text("保存选中的账号，保留已有账号及本地邮件。收发邮件时再连接邮箱验证凭据。").font(.caption).foregroundStyle(MailTheme.muted)
                        Button("保存 \(selected.count) 个账号"){save(preview)}.disabled(selected.isEmpty)
                        Button("重新下载"){self.preview=nil;selected=[];error=nil}
                    }
                }else{
                    Section("加密配置"){
                        TextField("HTTPS URL",text:$url).keyboardType(.URL).textInputAutocapitalization(.never).autocorrectionDisabled().textContentType(.URL)
                        Picker("加密方式",selection:$method){ForEach(ConfigCipher.allCases){Text($0.title).tag($0)}}
                        SecureField("解密密码",text:$password).textInputAutocapitalization(.never).autocorrectionDisabled()
                        Text("选择生成文件时使用的加密方式，下载后在本机解密。文件只包含密文串，解密密码不会上传或保存。").font(.caption).foregroundStyle(MailTheme.muted)
                        Button(busy ? "正在下载并解密…" : "下载并解密"){download()}.disabled(busy || url.isEmpty || password.isEmpty)
                        if busy{ProgressView().accessibilityLabel("正在下载并解密配置")}
                    }.disabled(busy)
                }
            }.navigationTitle("导入加密配置").navigationBarTitleDisplayMode(.inline)
            .toolbar{ToolbarItem(placement:.cancellationAction){Button("取消"){work?.cancel();password="";preview=nil;dismiss()}}}
            .onAppear{url=store.disk.read("config-source-url",as:String.self) ?? "";method=ConfigCipher(rawValue:store.disk.read("config-source-cipher",as:String.self) ?? "") ?? .aes256}
            .onDisappear{work?.cancel();password="";preview=nil}
        }
    }
    private func download(){
        busy=true;error=nil
        let address=url.trimmingCharacters(in:.whitespacesAndNewlines),secret=password,cipher=method
        work=Task{
            defer{busy=false;password=""}
            do{
                let result=try await RemoteConfig.fetch(address,password:secret,method:cipher)
                guard !Task.isCancelled else{return}
                url=address;preview=result;selected=Set(result.accounts.map(\.id))
            }catch{if !Task.isCancelled{self.error=error.localizedDescription}}
        }
    }
    private func save(_ preview:ConfigPreview){
        do{
            try store.importAccounts(preview.accounts.filter{selected.contains($0.id)})
            try? store.disk.write("config-source-url",url)
            try? store.disk.write("config-source-cipher",method.rawValue)
            store.notice="已导入 \(selected.count) 个账号，在邮件页选择邮箱并刷新即可读取。"
            self.preview=nil;dismiss()
        }catch{self.error="账号保存失败，请重试；原有账号仍保留。"}
    }
}
