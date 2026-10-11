import SwiftUI
import UniformTypeIdentifiers

struct RootView: View {
    @EnvironmentObject var store: MailStore
    @State private var editor: ComposeDraft?
    @State private var mailbox = false
    var body: some View {
        TabView {
            NavigationStack {
                VStack(spacing:0) {
                    if let error=store.error { StatusBanner(text:error){Task{await store.reload()}} }
                    if let notice=store.notice { StatusBanner(text:notice,actionLabel:"关闭"){store.notice=nil} }
                    if store.offline{Text("离线缓存 · 连接恢复后可刷新").font(.caption).foregroundStyle(MailTheme.warning).padding(MailTheme.space2)}
                    List {
                        if store.messages.isEmpty {
                            ContentUnavailableView(store.loading ? "正在连接邮箱" : "没有邮件",systemImage:"tray",description:Text(store.accounts.isEmpty ? "在“邮箱”中添加账号，直接收发邮件" : "下拉刷新，或切换文件夹和筛选条件"))
                        }
                        ForEach(store.messages) { message in
                            NavigationLink { MessageView(summary:message) } label: { MailRow(message:message) }
                        }
                        if store.hasMore{Button(store.loading ? "正在加载…" : "加载更多"){Task{await store.reload(more:true)}}.disabled(store.loading)}
                    }.listStyle(.plain).refreshable{await store.reload()}
                }
                .navigationTitle(store.folderTitle)
                .navigationBarTitleDisplayMode(.inline)
                .searchable(text:$store.search,prompt:"搜索邮件")
                .task(id:store.search){try? await Task.sleep(for:.milliseconds(350));guard !Task.isCancelled else{return};await store.reload()}
                .toolbar {
                    ToolbarItem(placement:.topBarLeading){Button{mailbox=true}label:{Label("选择文件夹",systemImage:"line.3.horizontal")}.disabled(store.accounts.isEmpty)}
                    ToolbarItemGroup(placement:.topBarTrailing){
                        Button{store.unread.toggle();Task{await store.reload()}}label:{Label(store.unread ? "显示全部" : "仅未读",systemImage:store.unread ? "envelope.badge" : "envelope")}
                        Button{editor=ComposeDraft(accountId:store.accountId)}label:{Label("写邮件",systemImage:"square.and.pencil")}.disabled(store.account==nil)
                    }
                }
                .overlay(alignment:.bottom){if store.loading{ProgressView().padding().background(.regularMaterial,in:Capsule())}}
            }.tabItem{Label("邮件",systemImage:"tray")}
            NavigationStack {
                List {
                    if store.localDrafts.isEmpty{ContentUnavailableView("没有本地草稿",systemImage:"doc",description:Text("编辑邮件时自动保存到本机"))}
                    ForEach(store.localDrafts){draft in Button{editor=draft}label:{VStack(alignment:.leading){Text(draft.title).foregroundStyle(MailTheme.text);Text(draft.to).font(.caption).foregroundStyle(MailTheme.muted)}}}
                        .onDelete{indices in let ids=indices.map{store.localDrafts[$0].id};for id in ids{do{try store.removeDraft(id)}catch{store.error=error.localizedDescription}}}
                }.navigationTitle("本地草稿")
            }.tabItem{Label("草稿",systemImage:"doc")}
            NavigationStack{OutboxView()}.tabItem{Label("发件记录",systemImage:"paperplane")}
            NavigationStack{AccountsView()}.tabItem{Label("邮箱",systemImage:"person.crop.circle")}
        }
        .sheet(item:$editor){ComposeView(initial:$0)}
        .sheet(isPresented:$mailbox){MailboxView()}
    }
}
struct StatusBanner: View {
    let text:String
    var actionLabel:String="重试"
    var retry:()->Void
    var body:some View{HStack(alignment:.top){Image(systemName:"info.circle");Text(text).font(.subheadline);Spacer();Button(actionLabel,action:retry).frame(minHeight:MailTheme.target)}.padding(.horizontal,MailTheme.space4).foregroundStyle(MailTheme.text).background(MailTheme.fill)}
}
struct MailRow:View {
    let message:MailMessage
    var body:some View {
        HStack(alignment:.top,spacing:MailTheme.space3){
            Text(String(message.sender.prefix(1)).uppercased()).font(.headline).frame(width:MailTheme.target,height:MailTheme.target).foregroundStyle(MailTheme.blueText).background(MailTheme.blueTint,in:Circle()).accessibilityHidden(true)
            VStack(alignment:.leading,spacing:MailTheme.space1){
                HStack{Text(message.sender).font(.subheadline).fontWeight(message.isRead ? .regular : .semibold).lineLimit(1);Spacer();Text(message.date,style:.date).font(.caption).foregroundStyle(MailTheme.muted)}
                Text(message.title).font(.body).fontWeight(message.isRead ? .regular : .semibold).lineLimit(2)
                HStack(spacing:MailTheme.space2){if !message.isRead{Text("未读").foregroundStyle(MailTheme.primary)};if message.isStarred{Image(systemName:"star.fill").foregroundStyle(MailTheme.warning)};Text(message.from.first?.email ?? "").lineLimit(1).foregroundStyle(MailTheme.muted)}.font(.caption)
            }
        }.padding(.vertical,MailTheme.space2).foregroundStyle(MailTheme.text)
    }
}
struct MailboxView:View {
    @EnvironmentObject var store:MailStore
    @Environment(\.dismiss) private var dismiss
    var body:some View {
        NavigationStack{
            List{
                Section("邮箱") {ForEach(store.accounts){account in Button{Task{await store.selectAccount(account.id)}}label:{HStack{Text(account.title);Spacer();if account.id==store.accountId{Image(systemName:"checkmark")}}}}}
                Section("文件夹") {ForEach(store.folders){folder in Button{store.folder=folder.path;dismiss();Task{await store.reload()}}label:{HStack{Text(folder.title);Spacer();if folder.path==store.folder{Image(systemName:"checkmark")}}}}}
            }.navigationTitle("邮箱与文件夹").toolbar{ToolbarItem(placement:.topBarTrailing){Button("完成"){dismiss()}}}
        }
    }
}
struct MessageView:View {
    @EnvironmentObject var store:MailStore
    @Environment(\.dismiss) private var dismiss
    let summary:MailMessage
    @State private var message:MailMessage?
    @State private var error:String?
    @State private var busy=false
    @State private var editor:ComposeDraft?
    @State private var deleting=false
    @State private var move=false
    @State private var exportURL:URL?
    @State private var retry=0
    var body:some View {
        VStack(spacing:0){
            if let error{StatusBanner(text:error){retry += 1}}
            if let mail=message {
                ScrollView{
                    VStack(alignment:.leading,spacing:MailTheme.space4){
                        Text(mail.title).font(.title2).fontWeight(.semibold)
                        VStack(alignment:.leading,spacing:MailTheme.space1){Text(mail.sender).font(.headline);Text(mail.from.map(\.email).joined(separator:", "));Text("收件人："+mail.to.map(\.email).joined(separator:", "));if !mail.cc.isEmpty{Text("抄送："+mail.cc.map(\.email).joined(separator:", "))};Text(mail.date.formatted()).foregroundStyle(MailTheme.muted)}.font(.caption).textSelection(.enabled)
                        Divider()
                        Text(mail.body?.isEmpty==false ? mail.body! : "（邮件没有可显示的正文）").font(.body).frame(maxWidth:.infinity,alignment:.leading).textSelection(.enabled)
                        if !mail.attachments.isEmpty {
                            Divider();Text("附件").font(.headline)
                            ForEach(mail.attachments){attachment in Button{export(attachment)}label:{HStack{Image(systemName:"paperclip");Text(attachment.filename);Spacer();Text(ByteCountFormatter.string(fromByteCount:Int64(attachment.size),countStyle:.file)).font(.caption)}}.frame(minHeight:MailTheme.target)}
                        }
                    }.padding(MailTheme.space4)
                }
                HStack{
                    Button("回复"){editor=store.reply(mail,all:false)}
                    Button("回复全部"){editor=store.reply(mail,all:true)}
                    Button("转发"){editor=ComposeDraft(accountId:mail.accountId,subject:"Fwd: "+mail.subject,text:"\n\n—— 转发邮件 ——\n"+(mail.body ?? ""),attachments:mail.attachments)}
                }.buttonStyle(.bordered).frame(minHeight:MailTheme.target).padding(MailTheme.space3)
                .disabled(busy)
            }else{Spacer();ProgressView("正在读取邮件");Spacer()}
        }.navigationTitle("邮件").navigationBarTitleDisplayMode(.inline)
        .task(id:retry){await load()}
        .toolbar{
            if let mail=message {
                ToolbarItemGroup(placement:.topBarTrailing){
                    Button{act{try await store.flag(mail,star:!mail.isStarred);message?.isStarred.toggle()}}label:{Label(mail.isStarred ? "取消星标" : "星标",systemImage:mail.isStarred ? "star.fill" : "star")}.disabled(busy)
                    Menu{
                        Button(mail.isRead ? "标为未读" : "标为已读"){act{try await store.flag(mail,read:!mail.isRead);message?.isRead.toggle()}}
                        Button("移动到文件夹"){move=true}
                        Button("移到已删除",role:.destructive){deleting=true}
                    }label:{Label("更多操作",systemImage:"ellipsis.circle")}.disabled(busy)
                }
            }
        }
        .sheet(item:$editor){ComposeView(initial:$0)}
        .sheet(isPresented:$move){NavigationStack{List(store.folders.filter{$0.path != summary.folder}){folder in Button(folder.title){move=false;act{try await store.move(summary,to:folder.path);dismiss()}}}.navigationTitle("移动到").toolbar{Button("取消"){move=false}}}}
        .confirmationDialog("将邮件移到已删除文件夹？",isPresented:$deleting,titleVisibility:.visible){Button("移到已删除",role:.destructive){
            guard let target=store.folders.first(where:{$0.role=="trash"}) else{error="邮箱没有已删除文件夹，使用移动功能选择目标";return}
            act{try await store.move(summary,to:target.path);dismiss()}
        }}
        .sheet(isPresented:Binding(get:{exportURL != nil},set:{if !$0{exportURL=nil}})){if let url=exportURL{ShareFile(url:url)}}
    }
    private func load()async {
        if message==nil{message=store.disk.read("message:"+summary.id,as:MailMessage.self)}
        do{let found=try await store.detail(summary);guard !Task.isCancelled else{return};message=found;error=nil
            if !found.isRead{do{try await store.flag(found,read:true);message?.isRead=true}catch{self.error="已读状态未同步，可在更多操作中重试"}}
        }catch{self.error=error.localizedDescription}
    }
    private func act(_ work:@escaping ()async throws->Void){busy=true;Task{defer{busy=false};do{try await work();error=nil}catch{self.error=error.localizedDescription}}}
    private func export(_ attachment:MailAttachment){
        guard let data=attachment.data else{error="附件尚未下载，重新读取邮件后重试";return}
        do{
            let dir=FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString,isDirectory:true)
            try FileManager.default.createDirectory(at:dir,withIntermediateDirectories:true)
            let name=URL(fileURLWithPath:attachment.filename).lastPathComponent
            let url=dir.appendingPathComponent(["",".",".."].contains(name) ? "attachment" : name)
            try data.write(to:url,options:[.atomic,.completeFileProtection]);exportURL=url
        }catch{self.error="无法导出附件，请重试"}
    }
}
struct ShareFile:UIViewControllerRepresentable {
    let url:URL
    func makeUIViewController(context:Context)->UIActivityViewController{UIActivityViewController(activityItems:[url],applicationActivities:nil)}
    func updateUIViewController(_ uiViewController:UIActivityViewController,context:Context){}
}

struct ComposeView:View {
    @EnvironmentObject var store:MailStore
    @Environment(\.dismiss) private var dismiss
    @State private var draft:ComposeDraft
    @State private var busy=false
    @State private var error:String?
    @State private var importer=false
    @State private var delivered=false
    @State private var discard=false
    init(initial:ComposeDraft){_draft=State(initialValue:initial)}
    var body:some View {
        NavigationStack{
            Form{
                if let error{Section{Text(error).foregroundStyle(MailTheme.danger)}}
                Section{
                    Picker("发件邮箱",selection:$draft.accountId){ForEach(store.accounts){Text($0.title).tag($0.id)}}.disabled(!draft.inReplyTo.isEmpty)
                    TextField("收件人（多个地址用逗号分隔）",text:$draft.to).keyboardType(.emailAddress).textInputAutocapitalization(.never).autocorrectionDisabled()
                    TextField("抄送",text:$draft.cc).keyboardType(.emailAddress).textInputAutocapitalization(.never).autocorrectionDisabled()
                    TextField("密送",text:$draft.bcc).keyboardType(.emailAddress).textInputAutocapitalization(.never).autocorrectionDisabled()
                    TextField("主题",text:$draft.subject)
                }
                Section("正文"){TextEditor(text:$draft.text).frame(minHeight:240).accessibilityLabel("邮件正文")}
                Section("附件"){
                    ForEach(draft.attachments){attachment in HStack{Text(attachment.filename);Spacer();Button("移除"){draft.attachments.removeAll{$0.id==attachment.id}}}}
                    Button("添加附件"){importer=true}
                    Text("邮件总大小上限 25 MB").font(.caption).foregroundStyle(MailTheme.muted)
                }
                Section{Text("草稿自动保存在本机。发送结果可在“发件记录”查看。").font(.caption).foregroundStyle(MailTheme.muted);Button("丢弃草稿",role:.destructive){discard=true}}
            }.disabled(busy)
            .navigationTitle(draft.inReplyTo.isEmpty ? "写邮件" : "回复邮件")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar{
                ToolbarItem(placement:.cancellationAction){Button("保存并关闭"){save();dismiss()}.disabled(busy)}
                ToolbarItem(placement:.confirmationAction){Button(busy ? "发送中…" : "发送"){send()}.disabled(busy || draft.accountId.isEmpty)}
            }
            .onChange(of:draft){_,_ in save()}
            .onDisappear{if !delivered{save()}}
            .interactiveDismissDisabled(busy)
            .fileImporter(isPresented:$importer,allowedContentTypes:[.item],allowsMultipleSelection:true,onCompletion:importFiles)
            .confirmationDialog("丢弃这封本地草稿及附件？",isPresented:$discard,titleVisibility:.visible){Button("丢弃草稿",role:.destructive){do{try store.removeDraft(draft.id);delivered=true;dismiss()}catch{self.error=error.localizedDescription}}}
        }
    }
    private func importFiles(_ result:Result<[URL],Error>){
        do{
            let urls=try result.get()
            for url in urls{
                let access=url.startAccessingSecurityScopedResource();defer{if access{url.stopAccessingSecurityScopedResource()}}
                let size=(try url.resourceValues(forKeys:[.fileSizeKey])).fileSize ?? 0
                guard size<=15*1024*1024 else{throw MailFailure.message("单个附件不能超过 15 MB")}
                let data=try Data(contentsOf:url)
                let existing=draft.attachments.reduce(0){total,item in total+(item.data?.count ?? 0)}
                guard existing+data.count<=18*1024*1024 else{throw MailFailure.message("附件总大小不能超过 18 MB")}
                let type=UTType(filenameExtension:url.pathExtension)?.preferredMIMEType ?? "application/octet-stream"
                draft.attachments.append(MailAttachment(id:UUID().uuidString,filename:url.lastPathComponent,mimeType:type,size:data.count,data:data))
            }
        }catch{self.error=error.localizedDescription}
    }
    private func save(){guard !delivered,!busy else{return};do{try store.saveDraft(draft)}catch{self.error="草稿保存失败，请保留编辑页面并重试"}}
    private func send(){
        do{try draft.validate()}catch{self.error=error.localizedDescription;return}
        busy=true
        Task{
            do{try await store.send(draft);delivered=true;busy=false;dismiss()}
            catch{busy=false;self.error=error.localizedDescription
                // Keep the persisted failed/uncertain job; edits become a new draft only after explicit user choice.
                delivered=true;dismiss();store.error=error.localizedDescription}
        }
    }
}
struct AccountsView:View {
    @EnvironmentObject var store:MailStore
    @State private var edit:MailAccount?
    @State private var remove:MailAccount?
    @State private var importing=false
    var body:some View {
        List{
            Button{importing=true}label:{Label("从 URL 导入加密配置",systemImage:"square.and.arrow.down")}.frame(minHeight:MailTheme.target)
            if store.accounts.isEmpty{ContentUnavailableView("添加你的邮箱",systemImage:"envelope",description:Text("直接连接 IMAP/SMTP。QQ、163 等邮箱需先开启协议并生成客户端授权码。"))}
            ForEach(store.accounts){account in
                VStack(alignment:.leading,spacing:MailTheme.space2){
                    Text(account.title).font(.headline);Text(account.email).font(.caption).foregroundStyle(MailTheme.muted)
                    HStack{Button("编辑"){edit=account};Spacer();Button("移除",role:.destructive){remove=account}}
                }.padding(.vertical,MailTheme.space2)
            }
            Section("隐私"){Text("邮箱凭据仅保存在本机 Keychain；邮件使用系统 TLS 直接连接邮箱服务器。正文不会加载追踪图片。").font(.caption)}
        }.navigationTitle("邮箱")
        .toolbar{Button{edit=MailAccount()}label:{Label("添加邮箱",systemImage:"plus")}}
        .sheet(item:$edit){AccountEditor(initial:$0)}
        .sheet(isPresented:$importing){ConfigImportView()}
        .confirmationDialog("移除邮箱及其本地缓存、草稿和发送记录？服务器邮件会保留。",isPresented:Binding(get:{remove != nil},set:{if !$0{remove=nil}}),titleVisibility:.visible){
            Button("移除邮箱",role:.destructive){if let account=remove{do{try store.removeAccount(account.id);if let next=store.accounts.first{Task{await store.selectAccount(next.id)}}}catch{store.error=error.localizedDescription}};remove=nil}
        }
    }
}
struct AccountEditor:View {
    @EnvironmentObject var store:MailStore
    @Environment(\.dismiss) private var dismiss
    @State private var account:MailAccount
    @State private var preset="自定义"
    @State private var imapPort:String
    @State private var smtpPort:String
    @State private var busy=false
    @State private var error:String?
    init(initial:MailAccount){_account=State(initialValue:initial);_imapPort=State(initialValue:String(initial.imapPort));_smtpPort=State(initialValue:String(initial.smtpPort))}
    var body:some View {
        NavigationStack{
            Form{
                if let error{Text(error).foregroundStyle(MailTheme.danger)}
                Section("账号"){
                    TextField("邮箱地址",text:$account.email).keyboardType(.emailAddress).textContentType(account.username.isEmpty ? .username : .emailAddress).textInputAutocapitalization(.never).autocorrectionDisabled()
                    TextField("显示名称",text:$account.displayName)
                    Picker("服务商",selection:$preset){ForEach(["自定义","QQ","163","126","Gmail","阿里云"],id:\.self){Text($0)}}
                    TextField("登录用户名（留空使用邮箱）",text:$account.username).textContentType(account.username.isEmpty ? nil : .username).textInputAutocapitalization(.never).autocorrectionDisabled()
                    SecureField(account.oauth2 ? "OAuth 访问令牌" : "密码 / 客户端授权码",text:$account.credential).textContentType(.password).textInputAutocapitalization(.never).autocorrectionDisabled()
                    Text("点击键盘上的“密码”，从系统中启用的密码管理器选择邮箱条目。客户端授权码需先在邮箱中生成并保存。").font(.caption).foregroundStyle(MailTheme.muted)
                    Toggle("使用已有 OAuth 访问令牌",isOn:$account.oauth2)
                    Text(account.oauth2 ? "令牌到期后需要手动更新；本版本不包含 OAuth 授权流程。" : "QQ、163、Gmail 等通常需要客户端授权码或应用专用密码。").font(.caption).foregroundStyle(MailTheme.muted)
                }
                Section("收信 IMAP"){
                    TextField("服务器",text:$account.imapHost).keyboardType(.URL).textInputAutocapitalization(.never).autocorrectionDisabled()
                    TextField("端口",text:$imapPort).keyboardType(.numberPad)
                    TextField("专用登录用户名（可留空）",text:Binding(get:{account.imapUsername ?? ""},set:{account.imapUsername=$0.isEmpty ? nil : $0})).textInputAutocapitalization(.never).autocorrectionDisabled()
                    Toggle("STARTTLS（通常端口 143）",isOn:$account.imapStartTLS)
                    Text("默认使用隐式 TLS（993）；始终验证服务器证书。").font(.caption).foregroundStyle(MailTheme.muted)
                }
                Section("发信 SMTP"){
                    TextField("服务器",text:$account.smtpHost).keyboardType(.URL).textInputAutocapitalization(.never).autocorrectionDisabled()
                    TextField("端口",text:$smtpPort).keyboardType(.numberPad)
                    TextField("专用登录用户名（可留空）",text:Binding(get:{account.smtpUsername ?? ""},set:{account.smtpUsername=$0.isEmpty ? nil : $0})).textInputAutocapitalization(.never).autocorrectionDisabled()
                    Toggle("STARTTLS（通常端口 587）",isOn:$account.smtpStartTLS)
                    Text("默认使用隐式 TLS（465）。不允许明文登录。").font(.caption).foregroundStyle(MailTheme.muted)
                }
            }.disabled(busy).navigationTitle("邮箱设置").navigationBarTitleDisplayMode(.inline)
            .toolbar{ToolbarItem(placement:.cancellationAction){Button("取消"){dismiss()}.disabled(busy)};ToolbarItem(placement:.confirmationAction){Button(busy ? "验证中…" : "验证并保存"){save()}.disabled(busy)}}
            .onChange(of:preset){_,value in account.preset(value);imapPort=String(account.imapPort);smtpPort=String(account.smtpPort)}
            .onChange(of:account.imapStartTLS){_,value in imapPort=value ? "143" : "993"}
            .onChange(of:account.smtpStartTLS){_,value in smtpPort=value ? "587" : "465"}
            .interactiveDismissDisabled(busy)
        }
    }
    private func save(){
        guard let incoming=Int(imapPort),let outgoing=Int(smtpPort) else{error="端口需要是数字";return}
        account.imapPort=incoming;account.smtpPort=outgoing
        account.email=account.email.trimmingCharacters(in:.whitespacesAndNewlines)
        account.imapHost=account.imapHost.trimmingCharacters(in:.whitespacesAndNewlines);account.smtpHost=account.smtpHost.trimmingCharacters(in:.whitespacesAndNewlines)
        busy=true;Task{defer{busy=false};do{try await store.saveAccount(account);dismiss()}catch{self.error=error.localizedDescription}}
    }
}
struct OutboxView:View {
    @EnvironmentObject var store:MailStore
    @State private var retry:ComposeDraft?
    @State private var busy=Set<String>()
    var body:some View {
        List{
            if store.outbox.isEmpty{ContentUnavailableView("没有发送记录",systemImage:"paperplane",description:Text("发送结果保存在本机；发送失败时可手动重试"))}
            ForEach(store.outbox){job in
                VStack(alignment:.leading,spacing:MailTheme.space2){
                    Text(job.title).font(.headline);Text(job.to).font(.caption)
                    Text(["sent":"已发送","sending":"正在发送","uncertain":"结果未知","failed":"发送失败"][job.delivery] ?? job.delivery).foregroundStyle(job.delivery=="sent" ? MailTheme.success : MailTheme.warning)
                    if !job.error.isEmpty{Text(job.error).font(.caption).foregroundStyle(MailTheme.muted)}
                    if job.delivery=="failed" || job.delivery=="uncertain"{Button("重试发送"){retry=job}.disabled(busy.contains(job.id))}
                }.padding(.vertical,MailTheme.space2)
            }
        }.navigationTitle("发件记录")
        .confirmationDialog(retry?.delivery=="uncertain" ? "服务器可能已收到邮件。确认已检查“已发送”文件夹，仍要重发？" : "重新发送这封邮件？",isPresented:Binding(get:{retry != nil},set:{if !$0{retry=nil}}),titleVisibility:.visible){
            Button("确认重发"){if let job=retry{busy.insert(job.id);Task{defer{busy.remove(job.id)};do{try await store.send(job,force:true)}catch{store.error=error.localizedDescription}}};retry=nil}
        }
    }
}
