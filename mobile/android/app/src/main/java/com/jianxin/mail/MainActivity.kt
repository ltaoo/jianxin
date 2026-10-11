package com.jianxin.mail

import android.app.Activity
import android.app.AlertDialog
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.content.Intent
import android.graphics.Typeface
import android.view.View
import android.view.ViewGroup
import android.view.Gravity
import android.view.WindowInsets
import android.view.autofill.AutofillManager
import android.widget.*
import android.text.Editable
import android.text.TextWatcher
import android.text.InputType
import android.view.inputmethod.EditorInfo
import android.net.Uri
import android.provider.OpenableColumns
import java.util.Date
import java.text.DateFormat
import java.util.concurrent.Executors
import java.io.ByteArrayOutputStream

class MainActivity : Activity() {
    private lateinit var repository: MailRepository
    private val worker = Executors.newSingleThreadExecutor()
    private val handler = Handler(Looper.getMainLooper())
    private var accountId = ""
    private var folder = "INBOX"
    private var query = ""
    private var unread = false
    private var folders = emptyList<Folder>()
    private var messages = emptyList<Message>()
    private var more = false
    private var offset = 0
    private var route = "inbox"
    private var generation = 0
    private var busy = false
    private var composeDraft: Draft? = null
    private var saving = false
    private var attachmentExport: Attachment? = null
    private var currentMessage: Message? = null
    private lateinit var root: LinearLayout
    private lateinit var content: LinearLayout
    private lateinit var statusView: TextView
    private lateinit var progress: ProgressBar
    private var adapter: MailAdapter? = null
    private val saveRunnable = Runnable { persistDraft() }

    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        try { repository = sharedRepository ?: MailRepository(applicationContext).also { sharedRepository = it } } catch (e: Exception) {
            val text = TextView(this).apply { this.text = MailRepository.friendly(e); setPadding(dp(24),dp(24),dp(24),dp(24)); textSize = 18f }
            setContentView(text); return
        }
        accountId = state?.getString("account") ?: repository.accounts.firstOrNull()?.id ?: ""
        folder = state?.getString("folder") ?: "INBOX"
        if (repository.accounts.none { it.id == accountId }) accountId = repository.accounts.firstOrNull()?.id ?: ""
        if (android.os.Build.VERSION.SDK_INT >= 33) onBackInvokedDispatcher.registerOnBackInvokedCallback(android.window.OnBackInvokedDispatcher.PRIORITY_DEFAULT) { goBack() }
        if (state?.getString("draft") != null) repository.drafts.firstOrNull { it.id == state.getString("draft") && it.delivery == "draft" }?.let { showComposer(it.copy()); return }
        showInbox(); refreshFolders()
    }
    override fun onSaveInstanceState(out: Bundle) { super.onSaveInstanceState(out); out.putString("account",accountId); out.putString("folder",folder); composeDraft?.let { out.putString("draft",it.id) } }
    override fun onPause() { if (::repository.isInitialized) persistDraft(); super.onPause() }
    override fun onDestroy() { handler.removeCallbacks(saveRunnable); worker.shutdown(); super.onDestroy() }
    @Deprecated("Native back dispatcher is used on API 33+") override fun onBackPressed() { goBack() }
    private fun goBack() { if (busy && route == "compose") { toast("正在发送邮件，等待结果后再退出"); return }; if (route == "inbox") finish() else { if (route == "account-edit") getSystemService(AutofillManager::class.java)?.cancel(); persistDraft(); composeDraft = null; showInbox() } }
    private fun dp(value: Int) = (value * resources.displayMetrics.density).toInt()
    private fun toast(text: String) { Toast.makeText(this,text,Toast.LENGTH_LONG).show() }
    private fun text(value: String, size: Float = 16f, color: Int = MailTheme.text) = TextView(this).apply { this.text = value; textSize = size; setTextColor(color); setPadding(dp(16),dp(8),dp(16),dp(8)) }
    private fun button(value: String, allowWhileBusy: Boolean = false, action: () -> Unit) = Button(this).apply { this.text = value; minHeight = dp(MailTheme.target); setTextColor(MailTheme.primary); setOnClickListener { if (busy && !allowWhileBusy) toast("正在处理邮箱操作，请稍候") else action() } }
    private fun row(vararg views: View) = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL; views.forEach { addView(it,LinearLayout.LayoutParams(0,ViewGroup.LayoutParams.WRAP_CONTENT,1f)) } }
    private fun page(title: String, name: String, back: Boolean = true) {
        generation++; route = name; adapter = null
        root = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setBackgroundColor(MailTheme.canvas) }
        if (android.os.Build.VERSION.SDK_INT >= 30) root.setOnApplyWindowInsetsListener { v, insets ->
            val bars = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout())
            v.setPadding(bars.left,bars.top,bars.right,bars.bottom); insets
        }
        val heading = text(title,20f).apply { setTypeface(typeface,Typeface.BOLD) }
        root.addView(if (back) row(button("返回") { goBack() },heading) else heading)
        progress = ProgressBar(this,null,android.R.attr.progressBarStyleHorizontal).apply { isIndeterminate = true; visibility = if (busy) View.VISIBLE else View.GONE }
        root.addView(progress,LinearLayout.LayoutParams(-1,dp(3)))
        statusView = text("",14f,MailTheme.muted).apply { visibility = View.GONE; setBackgroundColor(MailTheme.fill) }
        root.addView(statusView)
        content = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        root.addView(content,LinearLayout.LayoutParams(-1,0,1f)); setContentView(root); root.requestApplyInsets()
    }
    private fun status(value: String) { if (::statusView.isInitialized) { statusView.text = value; statusView.visibility = if (value.isBlank()) View.GONE else View.VISIBLE } }
    private fun <T> io(work: () -> T, failure: ((Exception) -> Unit)? = null, complete: (T) -> Unit) {
        if (busy) { toast("正在处理邮箱操作，请稍候"); return }
        busy = true; if (::progress.isInitialized) progress.visibility = View.VISIBLE
        val run = generation
        worker.execute {
            val result = runCatching(work)
            handler.post {
                if (isDestroyed) return@post
                busy = false; if (::progress.isInitialized) progress.visibility = View.GONE
                if (run != generation) { result.exceptionOrNull()?.let { toast(MailRepository.friendly(it as? Exception ?: Exception(it))) }; return@post }
                result.fold(onSuccess = complete, onFailure = { cause ->
                    val error = cause as? Exception ?: Exception(cause)
                    if (failure != null) failure(error) else { status(MailRepository.friendly(error)); toast("操作失败，可以重试") }
                })
            }
        }
    }
    private fun showInbox() {
        page(folders.firstOrNull { it.path == folder }?.title ?: "收件箱","inbox",false)
        composeDraft = null
        val account = repository.accounts.firstOrNull { it.id == accountId }
        content.addView(button(account?.title ?: "添加邮箱") { if (repository.accounts.isEmpty()) showAccountEditor(MailAccount()) else chooseMailbox() })
        val search = EditText(this).apply { hint = "搜索邮件"; setText(query); isSingleLine = true; imeOptions = EditorInfo.IME_ACTION_SEARCH; minHeight = dp(48); setPadding(dp(16),0,dp(16),0) }
        search.setOnEditorActionListener { _,action,_ -> if (action == EditorInfo.IME_ACTION_SEARCH) { query = search.text.toString(); reload(); true } else false }
        content.addView(row(search,button("搜索") { query = search.text.toString(); reload() }))
        content.addView(row(button("刷新") { refreshFolders() },button(if (unread) "显示全部" else "仅未读") { unread = !unread; reload() },button("写邮件") {
            if (accountId.isBlank()) showAccountEditor(MailAccount()) else showComposer(Draft(accountId = accountId))
        }))
        val list = ListView(this).apply { dividerHeight = dp(1); setBackgroundColor(MailTheme.canvas) }
        adapter = MailAdapter(); list.adapter = adapter
        list.setOnItemClickListener { _,_,position,_ -> messages.getOrNull(position)?.let(::showMessage) }
        content.addView(list,LinearLayout.LayoutParams(-1,0,1f))
        val load = button("加载更多") { if (more) reload(true) else toast("没有更多邮件") }; content.addView(load)
        root.addView(row(button("草稿") { showDrafts() },button("发件记录") { showOutbox() },button("邮箱") { showAccounts() }))
        if (accountId.isEmpty()) status("添加邮箱后可直接收发邮件") else {
            try { messages = repository.cachedList(repository.listKey(accountId,folder,query,unread)); adapter?.notifyDataSetChanged() } catch (e: Exception) { status(MailRepository.friendly(e)) }
        }
        if (messages.isEmpty() && accountId.isNotEmpty()) status("下拉操作请使用“刷新”；没有本地邮件时连接服务器读取")
    }
    private fun refreshFolders() {
        if (accountId.isEmpty()) return
        try { folders = repository.cachedFolders(accountId) } catch (_: Exception) { }
        io({ repository.folders(accountId) }) { found -> folders = found; if (found.none { it.path == folder }) folder = found.firstOrNull { it.role == "inbox" }?.path ?: "INBOX"; if (route == "inbox") reload() }
    }
    private fun reload(next: Boolean = false) {
        if (accountId.isEmpty()) return
        val id = accountId; val path = folder; val search = query; val onlyUnread = unread; val start = if (next) offset else 0
        val key = repository.listKey(id,path,search,onlyUnread)
        if (!next) { offset = 0; more = false; try { messages = repository.cachedList(key); adapter?.notifyDataSetChanged() } catch (_: Exception) { } }
        status("正在连接邮箱…")
        io({ repository.page(id,path,search,onlyUnread,start) }) { page ->
            messages = if (next) (messages + page.first).distinctBy { it.id } else page.first
            offset = start + 30; more = page.second
            try { repository.cacheList(id,key,messages) } catch (_: Exception) { toast("本地缓存保存失败") }
            adapter?.notifyDataSetChanged(); status(if (messages.isEmpty()) "没有符合条件的邮件" else if (more) "已加载 ${messages.size} 封，可继续加载" else "已同步 ${messages.size} 封邮件")
        }
    }
    private fun chooseMailbox() {
        AlertDialog.Builder(this).setTitle("选择邮箱").setItems(repository.accounts.map { it.title }.toTypedArray()) { _,i ->
            if (busy) { toast("等待当前操作完成"); return@setItems }
            accountId = repository.accounts[i].id; folder = "INBOX"; query = ""; folders = repository.cachedFolders(accountId); messages = emptyList(); showInbox()
            io({ repository.folders(accountId) }) { found -> folders = found; chooseFolder() }
        }.setNeutralButton("选择文件夹") { _,_ -> chooseFolder() }.show()
    }
    private fun chooseFolder() {
        if (folders.isEmpty()) { toast("先刷新邮箱文件夹"); return }
        AlertDialog.Builder(this).setTitle("选择文件夹").setItems(folders.map { it.title }.toTypedArray()) { _,i ->
            if (busy) { toast("等待当前操作完成"); return@setItems }; folder = folders[i].path; messages = emptyList(); showInbox(); reload()
        }.show()
    }
    private inner class MailAdapter : BaseAdapter() {
        override fun getCount() = messages.size
        override fun getItem(position: Int) = messages[position]
        override fun getItemId(position: Int) = position.toLong()
        override fun getView(position: Int, convertView: View?, parent: ViewGroup): View {
            val mail = messages[position]
            val view = (convertView as? LinearLayout) ?: LinearLayout(this@MainActivity).apply {
                orientation = LinearLayout.VERTICAL; setPadding(dp(16),dp(12),dp(16),dp(12))
                repeat(3) { addView(TextView(this@MainActivity)) }
            }
            (view.getChildAt(0) as TextView).apply { text = getString(R.string.mail_sender,if (mail.isRead) "" else "● ",mail.sender,if (mail.isStarred) "  ★" else ""); textSize = 14f; setTextColor(MailTheme.text); setTypeface(null,if (mail.isRead) Typeface.NORMAL else Typeface.BOLD); maxLines = 1 }
            (view.getChildAt(1) as TextView).apply { text = mail.title; textSize = 16f; setTextColor(MailTheme.text); maxLines = 2; setTypeface(null,if (mail.isRead) Typeface.NORMAL else Typeface.BOLD) }
            (view.getChildAt(2) as TextView).apply { text = DateFormat.getDateTimeInstance(DateFormat.SHORT,DateFormat.SHORT).format(Date(mail.date)); textSize = 12f; setTextColor(MailTheme.muted) }
            return view
        }
    }
    private fun showMessage(summary: Message) {
        page("邮件","reader"); currentMessage = summary
        status("正在读取邮件…")
        content.addView(button("重新读取") { if (!busy) showMessage(summary) })
        try { repository.cachedDetail(summary)?.let { renderMessage(it,true); status("显示本地缓存，正在检查最新状态…") } } catch (_: Exception) { }
        io({ repository.detail(summary) }) { found -> currentMessage = found.first; renderMessage(found.first,found.second) }
    }
    private fun renderMessage(mail: Message, offline: Boolean) {
        content.removeAllViews(); status(if (offline) "离线缓存，连接恢复后可重新读取" else "")
        val scroll = ScrollView(this); val reading = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        scroll.addView(reading); content.addView(scroll,LinearLayout.LayoutParams(-1,0,1f))
        reading.addView(text(mail.title,22f).apply { setTypeface(typeface,Typeface.BOLD); setTextIsSelectable(true) })
        reading.addView(text("${mail.sender}\n${mail.from.joinToString(", ") { it.email }}\n收件人：${mail.to.joinToString(", ") { it.email }}\n抄送：${mail.cc.joinToString(", ") { it.email }}\n${DateFormat.getDateTimeInstance().format(Date(mail.date))}",13f,MailTheme.muted).apply { setTextIsSelectable(true) })
        reading.addView(text(mail.body?.ifEmpty { "（邮件没有可显示的正文）" } ?: "（邮件没有可显示的正文）").apply { setTextIsSelectable(true) })
        mail.attachments.forEach { attachment -> reading.addView(button("附件：${attachment.filename}（${attachment.size / 1024} KB）") { exportAttachment(attachment) }) }
        content.addView(row(button("回复",true) { showComposer(repository.reply(mail,false)) },button("回复全部",true) { showComposer(repository.reply(mail,true)) },button("转发",true) { showComposer(Draft(accountId = mail.accountId,subject = "Fwd: ${mail.subject}",text = "\n\n—— 转发邮件 ——\n${mail.body ?: ""}",attachments = mail.attachments)) }))
        content.addView(row(button(if (mail.isStarred) "取消星标" else "星标") { io({ repository.flag(mail,star = !mail.isStarred) }) { updateCachedFlags(mail); renderMessage(mail,false) } },
            button("更多") { messageActions(mail) },button("重新读取") { if (!busy) showMessage(mail) }))
        if (!mail.isRead && !offline) io({ repository.flag(mail,read = true) }) { updateCachedFlags(mail) }
    }
    private fun updateCachedFlags(mail: Message) {
        messages = messages.map { if (it.id == mail.id) it.copy(isRead = mail.isRead,isStarred = mail.isStarred) else it }
        try { repository.cacheList(mail.accountId,repository.listKey(accountId,folder,query,unread),messages) } catch (_: Exception) { }
    }
    private fun messageActions(mail: Message) {
        AlertDialog.Builder(this).setTitle("邮件操作").setItems(arrayOf(if (mail.isRead) "标为未读" else "标为已读","移动到文件夹","移到已删除")) { _,which ->
            when (which) {
                0 -> io({ repository.flag(mail,read = !mail.isRead) }) { updateCachedFlags(mail); status("已更新读取状态") }
                1 -> { val targets = folders.filter { it.path != mail.folder }; AlertDialog.Builder(this).setTitle("移动到").setItems(targets.map { it.title }.toTypedArray()) { _,i -> moveMessage(mail,targets[i].path) }.show() }
                2 -> { val trash = folders.firstOrNull { it.role == "trash" }; if (trash == null) toast("邮箱没有已删除文件夹，使用移动选择目标") else AlertDialog.Builder(this).setTitle("移到已删除？").setMessage(mail.title).setPositiveButton("移到已删除") { _,_ -> moveMessage(mail,trash.path) }.setNegativeButton("取消",null).show() }
            }
        }.show()
    }
    private fun moveMessage(mail: Message, target: String) { io({ repository.move(mail,target) }) { messages = messages.filter { it.id != mail.id }; repository.cacheList(accountId,repository.listKey(accountId,folder,query,unread),messages); showInbox(); reload() } }
    private fun exportAttachment(attachment: Attachment) {
        if (attachment.data == null) { toast("附件尚未下载，重新读取邮件后重试"); return }
        attachmentExport = attachment
        startActivityForResult(Intent(Intent.ACTION_CREATE_DOCUMENT).apply { addCategory(Intent.CATEGORY_OPENABLE); type = attachment.mimeType; putExtra(Intent.EXTRA_TITLE,attachment.filename.substringAfterLast('/').substringAfterLast('\\')) },EXPORT)
    }
    private fun showDrafts() {
        page("本地草稿","drafts")
        val scroll = ScrollView(this); val list = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }; scroll.addView(list); content.addView(scroll)
        val drafts = repository.drafts.filter { it.delivery == "draft" }.sortedByDescending { it.date }
        if (drafts.isEmpty()) list.addView(text("没有本地草稿。编辑邮件时自动保存到本机。",16f,MailTheme.muted))
        drafts.forEach { draft -> list.addView(button("${draft.title}\n${draft.to}") { showComposer(draft.copy()) }); list.addView(button("丢弃草稿") { AlertDialog.Builder(this).setTitle("丢弃这封草稿及附件？").setPositiveButton("丢弃") { _,_ -> try { repository.removeDraft(draft.id); showDrafts() } catch (e: Exception) { status(MailRepository.friendly(e)) } }.setNegativeButton("取消",null).show() }) }
    }
    private fun showOutbox() {
        page("发件记录","outbox")
        val scroll = ScrollView(this); val list = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }; scroll.addView(list); content.addView(scroll)
        val jobs = repository.drafts.filter { it.delivery != "draft" }.sortedByDescending { it.date }
        if (jobs.isEmpty()) list.addView(text("没有发送记录。发送结果保存在本机。",16f,MailTheme.muted))
        jobs.forEach { job ->
            val label = mapOf("sent" to "已发送","sending" to "正在发送","uncertain" to "结果未知","failed" to "发送失败")[job.delivery] ?: job.delivery
            list.addView(text("${job.title}\n$label\n${job.error}",16f,if (job.delivery == "sent") MailTheme.success else MailTheme.warning))
            if (job.delivery in listOf("failed","uncertain")) list.addView(button("重试发送") {
                AlertDialog.Builder(this).setTitle("确认重发？").setMessage(if (job.delivery == "uncertain") "服务器可能已收到邮件。先检查已发送文件夹，再决定重发，避免重复发送。" else job.title)
                    .setPositiveButton("确认重发") { _,_ -> io({ repository.send(job,true) }) { toast(it); showOutbox() } }.setNegativeButton("取消",null).show()
            })
        }
    }
    private fun showAccounts() {
        page("邮箱","accounts")
        val scroll = ScrollView(this); val list = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }; scroll.addView(list); content.addView(scroll)
        list.addView(button("添加邮箱") { showAccountEditor(MailAccount()) })
        list.addView(button("从 URL 导入加密配置") { showConfigImport() })
        repository.accounts.forEach { account ->
            list.addView(text("${account.title}\n${account.email}",18f))
            list.addView(row(button("编辑") { showAccountEditor(account.copy()) },button("移除") {
                AlertDialog.Builder(this).setTitle("移除邮箱？").setMessage("移除此邮箱的本地缓存、草稿和发送记录，服务器邮件保留。")
                    .setPositiveButton("移除") { _,_ -> io({ repository.removeAccount(account.id) }) { if (accountId == account.id) { accountId = repository.accounts.firstOrNull()?.id ?: ""; messages = emptyList(); folders = emptyList() }; showAccounts() } }.setNegativeButton("取消",null).show()
            }))
        }
        list.addView(text("系统 TLS 直接连接邮箱服务器。凭据和缓存使用 Android Keystore 加密；正文不加载脚本或追踪图片。",14f,MailTheme.muted))
    }
    private fun input(parent: LinearLayout, label: String, value: String, type: Int = InputType.TYPE_CLASS_TEXT): EditText {
        parent.addView(text(label,14f,MailTheme.muted))
        return EditText(this).apply { importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO; setText(value); inputType = type; minHeight = dp(48); setPadding(dp(16),dp(8),dp(16),dp(8)); parent.addView(this); contentDescription = label }
    }
    private fun showConfigImport() {
        page("导入加密配置","config-import")
        val scroll = ScrollView(this); val form = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }; scroll.addView(form); content.addView(scroll)
        val source = input(form,"HTTPS URL",repository.configURL(),InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI)
        form.addView(text("加密方式",14f,MailTheme.muted))
        val method = Spinner(this).apply {
            adapter = ArrayAdapter(this@MainActivity,android.R.layout.simple_spinner_dropdown_item,ConfigCipher.entries.map { it.title })
            minimumHeight = dp(MailTheme.target); contentDescription = "加密方式"; form.addView(this); setSelection(ConfigCipher.entries.indexOf(repository.configCipher()))
        }
        val password = input(form,"解密密码","",InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD).apply { isSaveEnabled = false }
        form.addView(text("选择生成文件时使用的加密方式，下载后在本机解密。文件只包含密文串，解密密码不会上传或保存。",14f,MailTheme.muted))
        form.addView(button("下载并解密") {
            val address = source.text.toString().trim(); val secret = password.text.toString()
            val cipher = ConfigCipher.entries[method.selectedItemPosition]
            if (secret.isEmpty()) { status("请输入解密密码"); return@button }
            try { RemoteConfig.url(address) } catch (e: MailException) { status(e.message ?: "请输入 HTTPS URL"); return@button }
            password.setText("")
            io({ RemoteConfig.fetch(address,secret,cipher) }) { found -> showConfigPreview(address,found,cipher) }
            status("正在下载并解密…")
        })
    }
    private fun showConfigPreview(address: String, preview: ConfigPreview, method: ConfigCipher) {
        page("选择要导入的账号","config-preview")
        val scroll = ScrollView(this); val form = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }; scroll.addView(form); content.addView(scroll)
        val selected = preview.accounts.map { it.id }.toMutableSet()
        if (preview.accounts.isEmpty()) form.addView(text("没有可导入的账号，请按下方提示调整配置后重新加密。",16f,MailTheme.warning))
        preview.accounts.forEach { account ->
            val updating = repository.accounts.any { it.importIdentity == account.importIdentity }
            form.addView(CheckBox(this).apply {
                text = getString(R.string.config_account_preview,account.title,account.email,account.imapHost,account.imapPort,account.smtpHost,account.smtpPort,getString(if (updating) R.string.config_update_account else R.string.config_new_account))
                setTextColor(MailTheme.text); minHeight = dp(MailTheme.target); isChecked = true; setPadding(dp(16),dp(8),dp(16),dp(8))
                setOnCheckedChangeListener { _,checked -> if (checked) selected.add(account.id) else selected.remove(account.id) }
            })
        }
        preview.warnings.forEach { form.addView(text(it,14f,MailTheme.warning)) }
        form.addView(text("保存选中的账号，保留已有账号及本地邮件。收发邮件时再连接邮箱验证凭据。",14f,MailTheme.muted))
        form.addView(button("保存选中的账号") {
            val values = preview.accounts.filter { it.id in selected }
            if (values.isEmpty()) { status("请至少选择一个账号"); return@button }
            io({ repository.importAccounts(values); runCatching { repository.rememberConfigURL(address); repository.rememberConfigCipher(method) } }) {
                if (accountId.isBlank()) accountId = repository.accounts.firstOrNull()?.id ?: ""
                toast("已导入 ${values.size} 个账号，在邮件页选择邮箱并刷新即可读取")
                showAccounts()
            }
        })
        form.addView(button("重新下载") { showConfigImport() })
    }
    private fun showAccountEditor(account: MailAccount) {
        page("邮箱设置","account-edit")
        val scroll = ScrollView(this); val form = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }; scroll.addView(form); content.addView(scroll)
        val email = input(form,"邮箱地址",account.email,InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS)
        val display = input(form,"显示名称",account.displayName)
        val user = input(form,"登录用户名（留空使用邮箱）",account.username,InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS)
        val secret = input(form,"密码 / 客户端授权码 / OAuth 访问令牌",account.credential,InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD)
        email.id = R.id.account_email; user.id = R.id.account_login; secret.id = R.id.account_password
        secret.importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_YES
        secret.setAutofillHints(View.AUTOFILL_HINT_PASSWORD)
        fun configureCredentialAutofill() {
            val customLogin = user.text.isNotBlank()
            email.importantForAutofill = if (customLogin) View.IMPORTANT_FOR_AUTOFILL_NO else View.IMPORTANT_FOR_AUTOFILL_YES
            user.importantForAutofill = if (customLogin) View.IMPORTANT_FOR_AUTOFILL_YES else View.IMPORTANT_FOR_AUTOFILL_NO
            email.setAutofillHints(if (customLogin) View.AUTOFILL_HINT_EMAIL_ADDRESS else View.AUTOFILL_HINT_USERNAME)
            user.setAutofillHints(View.AUTOFILL_HINT_USERNAME)
        }
        configureCredentialAutofill()
        user.addTextChangedListener(object : TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) { }
            override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) { configureCredentialAutofill() }
            override fun afterTextChanged(s: Editable?) { }
        })
        form.addView(button("从密码管理器填入") {
            val manager = getSystemService(AutofillManager::class.java)
            if (manager == null || !manager.isEnabled) {
                status("在系统设置中启用“密码自动填充”并选择密码管理器，再返回此处重试")
            } else {
                secret.requestFocus()
                manager.requestAutofill(secret)
            }
        })
        form.addView(text("从已启用的密码管理器选择邮箱条目。客户端授权码需先在邮箱中生成并保存。",14f,MailTheme.muted))
        val oauth = Switch(this).apply { text = getString(R.string.use_oauth); isChecked = account.oauth2; minHeight = dp(48); form.addView(this) }
        form.addView(text("QQ、163 等先开启 IMAP/SMTP 并生成授权码。OAuth 令牌到期后需手动更新，本版本不包含授权流程。",14f,MailTheme.muted))
        val imap = input(form,"IMAP 服务器",account.imapHost,InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI)
        val imapPort = input(form,"IMAP 端口",account.imapPort.toString(),InputType.TYPE_CLASS_NUMBER)
        val imapUser = input(form,"IMAP 专用登录用户名（可留空）",account.imapUsername ?: "")
        val imapStart = Switch(this).apply { text = getString(R.string.imap_starttls); isChecked = account.imapStartTLS; minHeight = dp(48); form.addView(this) }
        val smtp = input(form,"SMTP 服务器",account.smtpHost,InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI)
        val smtpPort = input(form,"SMTP 端口",account.smtpPort.toString(),InputType.TYPE_CLASS_NUMBER)
        val smtpUser = input(form,"SMTP 专用登录用户名（可留空）",account.smtpUsername ?: "")
        val smtpStart = Switch(this).apply { text = getString(R.string.smtp_starttls); isChecked = account.smtpStartTLS; minHeight = dp(48); form.addView(this) }
        imapStart.setOnCheckedChangeListener { _,checked -> imapPort.setText(if (checked) "143" else "993") }; smtpStart.setOnCheckedChangeListener { _,checked -> smtpPort.setText(if (checked) "587" else "465") }
        form.addView(button("填写服务商预设") {
            val names = arrayOf("QQ","163","126","Gmail","阿里云")
            AlertDialog.Builder(this).setTitle("服务商").setItems(names) { _,i -> account.preset(names[i]); imap.setText(account.imapHost); smtp.setText(account.smtpHost); imapStart.isChecked = false; smtpStart.isChecked = false; imapPort.setText(getString(R.string.imap_default_port)); smtpPort.setText(getString(R.string.smtp_default_port)) }.show()
        })
        form.addView(text("默认隐式 TLS：IMAP 993 / SMTP 465。始终验证服务器证书，不允许明文登录。",14f,MailTheme.muted))
        form.addView(button("验证并保存") {
            account.email = email.text.toString().trim(); account.displayName = display.text.toString(); account.username = user.text.toString().trim(); account.credential = secret.text.toString(); account.oauth2 = oauth.isChecked
            account.imapHost = imap.text.toString().trim(); account.smtpHost = smtp.text.toString().trim(); account.imapPort = imapPort.text.toString().toIntOrNull() ?: 0; account.smtpPort = smtpPort.text.toString().toIntOrNull() ?: 0
            account.imapUsername = imapUser.text.toString().takeIf { it.isNotEmpty() }; account.smtpUsername = smtpUser.text.toString().takeIf { it.isNotEmpty() }
            io({ repository.saveAccount(account) }) { getSystemService(AutofillManager::class.java)?.commit(); accountId = account.id; folder = "INBOX"; folders = repository.cachedFolders(accountId); messages = emptyList(); showInbox(); reload() }
        })
    }
    private fun showComposer(initial: Draft) {
        page(if (initial.inReplyTo.isEmpty()) "写邮件" else "回复邮件","compose")
        composeDraft = initial
        val scroll = ScrollView(this); val form = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }; scroll.addView(form); content.addView(scroll)
        val selected = repository.accounts.firstOrNull { it.id == initial.accountId }
        form.addView(button("发件邮箱：${selected?.title ?: "选择邮箱"}") {
            if (initial.inReplyTo.isNotEmpty()) { toast("回复使用原邮箱发送"); return@button }
            AlertDialog.Builder(this).setTitle("发件邮箱").setItems(repository.accounts.map { it.title }.toTypedArray()) { _,i -> persistDraft(); initial.accountId = repository.accounts[i].id; showComposer(initial) }.show()
        })
        val to = input(form,"收件人（多个地址用逗号分隔）",initial.to,InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS)
        val cc = input(form,"抄送",initial.cc,InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS)
        val bcc = input(form,"密送",initial.bcc,InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS)
        val subject = input(form,"主题",initial.subject)
        val body = input(form,"正文",initial.text,InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_MULTI_LINE or InputType.TYPE_TEXT_FLAG_CAP_SENTENCES).apply { minLines = 10; gravity = Gravity.TOP }
        val watcher = object : TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) { }
            override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {
                initial.to = to.text.toString(); initial.cc = cc.text.toString(); initial.bcc = bcc.text.toString(); initial.subject = subject.text.toString(); initial.text = body.text.toString()
                handler.removeCallbacks(saveRunnable); handler.postDelayed(saveRunnable,350)
            }
            override fun afterTextChanged(s: Editable?) { }
        }
        listOf(to,cc,bcc,subject,body).forEach { it.addTextChangedListener(watcher) }
        initial.attachments.forEach { attachment -> form.addView(row(text(attachment.filename,14f),button("移除") { initial.attachments = initial.attachments.filter { it.id != attachment.id }; persistDraft(); showComposer(initial) })) }
        form.addView(button("添加附件") { persistDraft(); startActivityForResult(Intent(Intent.ACTION_OPEN_DOCUMENT).apply { addCategory(Intent.CATEGORY_OPENABLE); type = "*/*"; putExtra(Intent.EXTRA_ALLOW_MULTIPLE,true) },IMPORT) })
        form.addView(text("草稿自动保存到本机。单个附件上限 15 MB，附件合计上限 18 MB。",14f,MailTheme.muted))
        form.addView(row(button("保存并关闭") { persistDraft(); composeDraft = null; showDrafts() },button("发送") {
            try { initial.validate(); persistDraft() } catch (e: Exception) { status(MailRepository.friendly(e)); return@button }
            if (busy) return@button
            // Detach the editor before delivery so onPause/autosave cannot overwrite its durable status.
            handler.removeCallbacks(saveRunnable); composeDraft = null; disableTree(form)
            io({ repository.send(initial.copy()) }, failure = { error ->
                toast(MailRepository.friendly(error))
                if (repository.drafts.any { it.id == initial.id && it.delivery != "draft" }) showOutbox() else showComposer(initial)
                status(MailRepository.friendly(error))
            }) { outcome -> toast(outcome); showOutbox() }
            status("正在发送；完成后可在发件记录查看结果")
        }))
        form.addView(button("丢弃草稿") { AlertDialog.Builder(this).setTitle("丢弃此本地草稿及附件？").setPositiveButton("丢弃") { _,_ -> handler.removeCallbacks(saveRunnable); repository.removeDraft(initial.id); composeDraft = null; showDrafts() }.setNegativeButton("取消",null).show() })
    }
    private fun disableTree(view: View) { view.isEnabled = false; if (view is ViewGroup) for (i in 0 until view.childCount) disableTree(view.getChildAt(i)) }
    private fun persistDraft() {
        handler.removeCallbacks(saveRunnable)
        val draft = composeDraft ?: return
        if (saving || draft.delivery != "draft") return
        saving = true
        try { repository.saveDraft(draft.copy()) } catch (_: Exception) { status("草稿保存失败，保留编辑页面后重试") } finally { saving = false }
    }
    @Deprecated("Activity result API kept dependency-free") override fun onActivityResult(request: Int, result: Int, data: Intent?) {
        super.onActivityResult(request,result,data)
        if (result != RESULT_OK || data == null) return
        if (request == IMPORT) {
            val draft = composeDraft ?: return
            val uris = data.clipData?.let { clip -> (0 until clip.itemCount).map { clip.getItemAt(it).uri } } ?: listOfNotNull(data.data)
            io({
                var attached = draft.attachments
                uris.forEach { uri ->
                    var name = "attachment"
                    contentResolver.query(uri,arrayOf(OpenableColumns.DISPLAY_NAME,OpenableColumns.SIZE),null,null,null)?.use { cursor -> if (cursor.moveToFirst()) {
                        name = cursor.getString(0) ?: name; if (!cursor.isNull(1) && cursor.getLong(1) > 15 * 1024 * 1024) throw MailException("单个附件不能超过 15 MB")
                    } }
                    val bytes = contentResolver.openInputStream(uri)?.use { stream ->
                        val out = ByteArrayOutputStream(); val buffer = ByteArray(8192)
                        while (true) { val n = stream.read(buffer); if (n < 0) break; if (out.size() + n > 15 * 1024 * 1024) throw MailException("单个附件不能超过 15 MB"); out.write(buffer,0,n) }; out.toByteArray()
                    } ?: throw MailException("无法读取附件")
                    if (attached.sumOf { it.data?.size ?: 0 } + bytes.size > 18 * 1024 * 1024) throw MailException("附件总大小不能超过 18 MB")
                    attached = attached + Attachment(filename = name,mimeType = contentResolver.getType(uri) ?: "application/octet-stream",size = bytes.size,data = bytes)
                }
                attached
            }) { attached -> draft.attachments = attached; persistDraft(); showComposer(draft) }
        } else if (request == EXPORT) {
            val attachment = attachmentExport ?: return; val uri = data.data ?: return
            io({ contentResolver.openOutputStream(uri)?.use { it.write(attachment.data ?: byteArrayOf()) } ?: throw MailException("无法保存附件") }) { toast("附件已保存"); attachmentExport = null }
        }
    }
    companion object { @Volatile private var sharedRepository: MailRepository? = null; private const val IMPORT = 40; private const val EXPORT = 41 }
}
