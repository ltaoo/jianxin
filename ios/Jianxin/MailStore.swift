import Foundation
import SwiftUI

@MainActor final class MailStore: ObservableObject {
    @Published var accounts = CredentialStore.load()
    @Published var folders: [MailFolder] = []
    @Published var messages: [MailMessage] = []
    @Published var drafts: [ComposeDraft] = []
    @Published var accountId = ""
    @Published var folder = "INBOX"
    @Published var search = ""
    @Published var unread = false
    @Published var loading = false
    @Published var hasMore = false
    @Published var offline = false
    @Published var error: String?
    @Published var notice: String?
    private var generation = UUID()
    private var offset = 0
    let disk = LocalStore()
    var account: MailAccount? { accounts.first{$0.id==accountId} }
    var folderTitle: String { folders.first{$0.path==folder}?.title ?? "收件箱" }
    var cacheKey: String { "list:\(accountId):\(folder):\(search):\(unread)" }
    var localDrafts: [ComposeDraft] { drafts.filter{$0.delivery=="draft"} }
    var outbox: [ComposeDraft] { drafts.filter{$0.delivery != "draft"} }

    func bootstrap() async {
        drafts=disk.read("drafts",as:[ComposeDraft].self) ?? []
        // A killed process during SMTP DATA must never silently resend.
        for i in drafts.indices where drafts[i].delivery=="sending" {drafts[i].delivery="uncertain";drafts[i].error=MailFailure.uncertain.localizedDescription}
        try? disk.write("drafts",drafts)
        if let first=accounts.first{await selectAccount(first.id)}
    }
    func saveAccount(_ value:MailAccount) async throws {
        try value.validate()
        // Verify IMAP credentials before saving; SMTP errors remain visible during sending.
        let found=try await mailWork { try IMAPClient(account:value).folders() }
        var next=accounts.filter{$0.id != value.id};next.append(value)
        try CredentialStore.save(next);accounts=next
        try disk.write("folders:"+value.id,found)
        await selectAccount(value.id)
    }
    func importAccounts(_ values:[MailAccount])throws {
        guard !values.isEmpty else{throw ConfigImportError.message("没有可导入的账号")}
        for value in values{try value.validate()}
        let next=MailAccount.merge(values,into:accounts)
        try CredentialStore.save(next);accounts=next
        if accountId.isEmpty{accountId=next.first?.id ?? ""}
    }
    func removeAccount(_ id:String)throws {
        guard !drafts.contains(where:{$0.accountId==id && $0.delivery=="sending"}) else{throw MailFailure.message("正在发送邮件，完成后再移除邮箱")}
        let next=accounts.filter{$0.id != id};try CredentialStore.save(next);accounts=next
        let retained=drafts.filter{$0.accountId != id};try disk.write("drafts",retained);drafts=retained
        // All per-account cache records are registered and removed with the account.
        let keys=disk.read("keys:"+id,as:[String].self) ?? []
        keys.forEach{disk.remove($0)};disk.remove("keys:"+id);disk.remove("folders:"+id)
        generation=UUID();messages=[];folders=[];accountId=next.first?.id ?? "";loading=false
    }
    private func cache<T:Encodable>(_ key:String,_ value:T,account:String)throws {
        try disk.write(key,value)
        var keys=disk.read("keys:"+account,as:[String].self) ?? []
        if !keys.contains(key){keys.append(key);try disk.write("keys:"+account,keys)}
    }
    func selectAccount(_ id:String) async {
        generation=UUID();let run=generation
        accountId=id;search="";folders=disk.read("folders:"+id,as:[MailFolder].self) ?? []
        folder=folders.first{$0.role=="inbox"}?.path ?? "INBOX"
        messages=disk.read(cacheKey,as:[MailMessage].self) ?? [];hasMore=false;offset=0;loading=true
        guard let selected=account else{loading=false;return}
        do{
            let found=try await mailWork{try IMAPClient(account:selected).folders()}
            guard run==generation else{return};folders=found;try disk.write("folders:"+id,found)
        }catch{guard run==generation else{return};offline=true;loading=false;self.error=error.localizedDescription;return}
        folder=folders.first{$0.role=="inbox"}?.path ?? "INBOX"
        await reload()
    }
    func reload(more:Bool=false) async {
        guard let selected=account,!more || !loading else{return}
        let run=UUID();generation=run
        let key=cacheKey,path=folder,query=search,onlyUnread=unread,start=more ? offset : 0
        if !more{messages=disk.read(key,as:[MailMessage].self) ?? [];hasMore=false;offset=0}
        loading=true
        defer{if run==generation{loading=false}}
        do{
            let page=try await mailWork{try IMAPClient(account:selected).page(folder:path,query:query,unread:onlyUnread,offset:start)}
            guard run==generation,!Task.isCancelled else{return}
            messages=more ? messages+page.messages.filter{next in !messages.contains{$0.id==next.id}} : page.messages
            offset=start+30;hasMore=page.more;offline=false;error=nil
            try cache(key,messages,account:selected.id)
        }catch{guard run==generation,!Task.isCancelled else{return};offline=true;self.error=error.localizedDescription}
    }
    func detail(_ summary:MailMessage) async throws -> MailMessage {
        guard let selected=accounts.first(where:{$0.id==summary.accountId}) else{throw MailFailure.message("邮箱已移除")}
        let key="message:"+summary.id
        do{
            let message=try await mailWork{try IMAPClient(account:selected).detail(summary)}
            try cache(key,message,account:selected.id);return message
        }catch{if let cached=disk.read(key,as:MailMessage.self){return cached};throw error}
    }
    func flag(_ message:MailMessage,read:Bool?=nil,star:Bool?=nil)async throws {
        guard let selected=accounts.first(where:{$0.id==message.accountId}) else{throw MailFailure.message("邮箱已移除")}
        try await mailWork {
            let imap=try IMAPClient(account:selected)
            if let read{try imap.flag(message,flag:"\\Seen",value:read)}
            if let star{try imap.flag(message,flag:"\\Flagged",value:star)}
        }
        if let index=messages.firstIndex(where:{$0.id==message.id}) {
            if let read{messages[index].isRead=read};if let star{messages[index].isStarred=star}
            try cache(cacheKey,messages,account:selected.id)
        }
        var cached=disk.read("message:"+message.id,as:MailMessage.self) ?? message
        if let read{cached.isRead=read};if let star{cached.isStarred=star}
        try cache("message:"+message.id,cached,account:selected.id)
    }
    func move(_ message:MailMessage,to target:String)async throws {
        guard let selected=accounts.first(where:{$0.id==message.accountId}) else{throw MailFailure.message("邮箱已移除")}
        try await mailWork{try IMAPClient(account:selected).move(message,to:target)}
        messages.removeAll{$0.id==message.id};try cache(cacheKey,messages,account:selected.id)
        disk.remove("message:"+message.id)
    }
    func saveDraft(_ draft:ComposeDraft)throws {
        var next=drafts.filter{$0.id != draft.id};if draft.hasContent{next.insert(draft,at:0)}
        try disk.write("drafts",next);drafts=next
    }
    func removeDraft(_ id:String)throws{let next=drafts.filter{$0.id != id};try disk.write("drafts",next);drafts=next}
    func send(_ draft:ComposeDraft,force:Bool=false)async throws {
        guard let selected=accounts.first(where:{$0.id==draft.accountId}) else{throw MailFailure.message("选择一个邮箱")}
        try draft.validate()
        let current=drafts.first{$0.id==draft.id}
        guard current?.delivery != "sending" && current?.delivery != "sent" else{throw MailFailure.message("此邮件正在发送或已发送")}
        if current?.delivery=="uncertain" && !force{throw MailFailure.uncertain}
        var job=draft;job.delivery="sending";job.error="";try saveDraft(job)
        let bytes:Data
        do{bytes=try await mailWork{try SMTPClient.send(draft,account:selected)}}catch{
            job.delivery=(error as? MailFailure).map{if case .uncertain=$0{return "uncertain"};return "failed"} ?? "failed"
            job.error=error.localizedDescription
            do{try saveDraft(job)}catch{drafts=drafts.filter{$0.id != job.id}+[job]}
            throw error
        }
        job.delivery="sent"
        do{try saveDraft(job)}catch{
            drafts=drafts.filter{$0.id != job.id}+[job]
            notice="邮件已发送；本地记录保存失败，重启后请先检查已发送邮件"
            return
        }
        notice="邮件已发送"
        let sentFolders=(disk.read("folders:"+selected.id,as:[MailFolder].self) ?? []).filter{$0.role=="sent"}
        if let sent=sentFolders.first {
            do{try await mailWork{try IMAPClient(account:selected).append(bytes,to:sent.path)}}
            catch{notice="邮件已发送；已发送文件夹副本保存失败"}
        }
    }

    func reply(_ message:MailMessage,all:Bool)->ComposeDraft {
        let own=accounts.first{$0.id==message.accountId}?.email.lowercased() ?? ""
        let recipients=(message.replyTo.isEmpty ? message.from : message.replyTo).filter{$0.email.lowercased() != own}
        var seen=Set(recipients.map{$0.email.lowercased()});seen.insert(own)
        let copies=all ? (message.to+message.cc).filter{seen.insert($0.email.lowercased()).inserted} : []
        var refs=message.references;if !message.messageId.isEmpty && !refs.contains(message.messageId){refs.append(message.messageId)}
        return ComposeDraft(accountId:message.accountId,to:recipients.map(\.email).joined(separator:", "),cc:copies.map(\.email).joined(separator:", "),
            subject:message.subject.lowercased().hasPrefix("re:") ? message.subject : "Re: "+message.subject,
            text:"\n\n—— \(message.sender)，\(message.date.formatted()) ——\n"+(message.body ?? ""),inReplyTo:message.messageId,references:refs)
    }
}
