package com.jianxin.mail

import androidx.appcompat.app.AppCompatActivity
import androidx.appcompat.content.res.AppCompatResources
import androidx.activity.OnBackPressedCallback
import androidx.core.view.GravityCompat
import androidx.drawerlayout.widget.DrawerLayout
import androidx.appcompat.widget.PopupMenu
import com.google.android.material.appbar.MaterialToolbar
import com.google.android.material.button.MaterialButton
import com.google.android.material.card.MaterialCardView
import com.google.android.material.chip.Chip
import com.google.android.material.chip.ChipGroup
import com.google.android.material.dialog.MaterialAlertDialogBuilder
import com.google.android.material.floatingactionbutton.ExtendedFloatingActionButton
import com.google.android.material.floatingactionbutton.FloatingActionButton
import com.google.android.material.progressindicator.LinearProgressIndicator
import com.google.android.material.textfield.TextInputLayout
import com.google.android.material.textfield.TextInputEditText
import com.google.android.material.materialswitch.MaterialSwitch
import com.google.android.material.checkbox.MaterialCheckBox
import android.content.res.ColorStateList
import android.graphics.drawable.GradientDrawable
import android.text.TextUtils
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
import java.util.concurrent.Executors
import java.util.concurrent.Future
import java.io.ByteArrayOutputStream

class MainActivity : AppCompatActivity() {
    private lateinit var repository: MailRepository
    private val material by lazy { MailUi(this) }
    private val worker = Executors.newFixedThreadPool(3)
    private val storageWorker = Executors.newSingleThreadExecutor()
    private val handler = Handler(Looper.getMainLooper())
    private var accountId = ""
    private var folder = "INBOX"
    private var query = ""
    private var unread = false
    private var folders = emptyList<Folder>()
    private var messages = emptyList<Message>()
    private var threads = emptyList<MailThread>()
    private var conversationMessages = emptyList<Message>()
    private val expandedMessages = mutableSetOf<String>()
    private val readerDetails = mutableMapOf<String,Message>()
    private val offlineMessages = mutableSetOf<String>()
    private var readerNotice = ""
    private var timeStyle = MailTime.DEFAULT
    private var more = false
    private var offset = 0
    private var unified = true
    private var unifiedInbox: UnifiedInbox? = null
    private var inboxErrors = emptyMap<String,String>()
    private val mailboxErrors = mutableMapOf<String,String>()
    private var route = "inbox"
    @Volatile private var generation = 0
    private var operation = 0
    private var cancellableSync = false
    private var busy = false
    private var activeRead: MailCancellation? = null
    private var activeWork: Future<*>? = null
    private var retainedInbox: InboxPage? = null
    private var composeDraft: Draft? = null
    private var saving = false
    private var attachmentExport: Attachment? = null
    private var currentMessage: Message? = null
    private var configFileUri: Uri? = null
    private var configLocalFile = false
    private var oauthPollRunnable: Runnable? = null
    private lateinit var root: LinearLayout
    private lateinit var content: LinearLayout
    private lateinit var statusView: TextView
    private lateinit var statusContainer: View
    private var dismissedInboxStatus: String? = null
    private lateinit var progress: ProgressBar
    private lateinit var toolbar: MaterialToolbar
    private var drawer: DrawerLayout? = null
    private var loadMoreButton: MaterialButton? = null
    private var adapter: MailAdapter? = null
    private val saveRunnable = Runnable { persistDraft() }

    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        try { repository = sharedRepository ?: MailRepository(applicationContext).also { sharedRepository = it } } catch (e: Exception) {
            val text = TextView(this).apply { this.text = MailRepository.friendly(e); setPadding(dp(24),dp(24),dp(24),dp(24)); textSize = 18f }
            setContentView(text); return
        }
        accountId = state?.getString("account") ?: repository.accounts.firstOrNull()?.id ?: ""
        timeStyle = repository.timeFormat()
        folder = state?.getString("folder") ?: "INBOX"
        unified = state?.getBoolean("unified",true) ?: true
        if (repository.accounts.none { it.id == accountId }) accountId = repository.accounts.firstOrNull()?.id ?: ""
        onBackPressedDispatcher.addCallback(this,object : OnBackPressedCallback(true) { override fun handleOnBackPressed() { goBack() } })
        if (state?.getString("draft") != null) repository.drafts.firstOrNull { it.id == state.getString("draft") && it.delivery == "draft" }?.let { showComposer(it.copy()); return }
        if (state?.getBoolean("configLocalFile") == true) {
            showConfigImport(localFile = true,file = state.getString("configFileUri")?.let { Uri.parse(it) }); return
        }
        showInbox(); refreshFolders()
    }
    override fun onSaveInstanceState(out: Bundle) { super.onSaveInstanceState(out); out.putString("account",accountId); out.putString("folder",folder); out.putBoolean("unified",unified); composeDraft?.let { out.putString("draft",it.id) }; if (route == "config-import" && configLocalFile) { out.putBoolean("configLocalFile",true); out.putString("configFileUri",configFileUri?.toString()) } }
    override fun onPause() { if (::repository.isInitialized) persistDraft(); super.onPause() }
    override fun onDestroy() { if (cancellableSync) cancelSync(); retainedInbox = null; handler.removeCallbacks(saveRunnable); oauthPollRunnable?.let { handler.removeCallbacks(it) }; worker.shutdown(); storageWorker.shutdown(); super.onDestroy() }
    private fun goBack() {
        if (drawer?.isDrawerOpen(GravityCompat.START) == true) { drawer?.closeDrawer(GravityCompat.START); return }
        if (busy && route == "compose") { toast("正在发送邮件，等待结果后再退出"); return }
        if (cancellableSync) cancelSync()
        if (route == "inbox" && unified) finish()
        else if (route == "inbox") openUnifiedInbox()
        else if (route == "provider-picker") showAccounts()
        else if (route == "account-new") { getSystemService(AutofillManager::class.java)?.cancel(); showProviderPicker() }
        else { if (route == "account-edit") getSystemService(AutofillManager::class.java)?.cancel(); persistDraft(); composeDraft = null; showInbox() }
    }
    private fun dp(value: Int) = (value * resources.displayMetrics.density).toInt()
    private fun toast(text: String) { Toast.makeText(this,text,Toast.LENGTH_LONG).show() }
    private fun text(value: String, size: Float = 16f, color: Int = MailTheme.text) = TextView(this).apply { this.text = value; textSize = size; setTextColor(color); setPadding(dp(16),dp(8),dp(16),dp(8)) }
    private fun button(value: String, allowWhileBusy: Boolean = false, action: () -> Unit) = MaterialButton(this,null,com.google.android.material.R.attr.materialButtonOutlinedStyle).apply {
        text = value; isAllCaps = false; minHeight = dp(MailTheme.target); cornerRadius = dp(24); setTextColor(MailTheme.primary)
        insetTop = dp(4); insetBottom = dp(4)
        setOnClickListener { if (busy && !allowWhileBusy) toast("正在处理邮箱操作，请稍候") else action() }
    }
    private fun row(vararg views: View) = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL; views.forEach { addView(it,LinearLayout.LayoutParams(0,ViewGroup.LayoutParams.WRAP_CONTENT,1f)) } }
    private fun action(value: String, icon: Int = 0, kind: MailUi.Action = MailUi.Action.Text, allowWhileBusy: Boolean = false, run: () -> Unit) = material.action(value,icon,kind).apply {
        setOnClickListener { if (busy && !allowWhileBusy) toast("正在处理邮箱操作，请稍候") else run() }
    }
    private fun footer(view: View) {
        val bar = LinearLayout(this).apply { gravity = Gravity.END or Gravity.CENTER_VERTICAL; setPadding(dp(16),dp(8),dp(16),dp(12)); setBackgroundColor(MailTheme.canvas) }
        bar.addView(view,LinearLayout.LayoutParams(-1,-2)); content.addView(bar)
    }
    private fun toolbarAction(label: String, icon: Int, run: () -> Unit) {
        toolbar.menu.add(label).apply {
            this.icon = AppCompatResources.getDrawable(this@MainActivity,icon)?.mutate()?.apply { setTint(MailTheme.primary) }
            setShowAsAction(android.view.MenuItem.SHOW_AS_ACTION_ALWAYS)
            setOnMenuItemClickListener { if (cancellableSync) cancelSync(); if (busy) toast("正在处理邮箱操作，请稍候") else run(); true }
        }
    }
    private fun popupItem(popup: PopupMenu, label: String, icon: Int, destructive: Boolean = false, run: () -> Unit) {
        popup.menu.add(label).apply {
            this.icon = AppCompatResources.getDrawable(this@MainActivity,icon)?.mutate()?.apply { setTint(if (destructive) MailTheme.danger else MailTheme.muted) }
            setOnMenuItemClickListener { if (busy) toast("正在处理邮箱操作，请稍候") else run(); true }
        }
    }
    private fun moreAction(build: (PopupMenu) -> Unit) {
        val id = View.generateViewId()
        toolbar.menu.add(0,id,10,"更多").apply {
            setIcon(R.drawable.ic_more); setShowAsAction(android.view.MenuItem.SHOW_AS_ACTION_ALWAYS)
            setOnMenuItemClickListener { val popup = PopupMenu(this@MainActivity,toolbar.findViewById<View>(id) ?: toolbar); build(popup); popup.setForceShowIcon(true); popup.show(); true }
        }
    }
    private fun page(title: String, name: String, back: Boolean = true) {
        if (cancellableSync) cancelSync()
        if (name != "reader") retainedInbox = null
        oauthPollRunnable?.let { handler.removeCallbacks(it) }; oauthPollRunnable = null
        generation++; route = name; adapter = null; drawer = null; loadMoreButton = null
        root = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setBackgroundColor(MailTheme.canvas) }
        if (android.os.Build.VERSION.SDK_INT >= 30) root.setOnApplyWindowInsetsListener { v, insets ->
            val bars = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout())
            v.setPadding(bars.left,bars.top,bars.right,bars.bottom); insets
        }
        toolbar = MaterialToolbar(this).apply {
            this.title = title; setTitleTextColor(MailTheme.text); setSubtitleTextColor(MailTheme.muted)
            setBackgroundColor(MailTheme.canvas); setNavigationIcon(if (back) R.drawable.ic_back else R.drawable.ic_menu)
            setNavigationIconTint(MailTheme.text); navigationContentDescription = if (back) "返回" else "打开邮箱抽屉"
            setNavigationOnClickListener { if (back) goBack() else openMailboxDrawer() }
        }
        root.addView(toolbar,LinearLayout.LayoutParams(-1,dp(64)))
        progress = LinearProgressIndicator(this).apply { isIndeterminate = true; setIndicatorColor(MailTheme.primary); visibility = if (busy) View.VISIBLE else View.GONE }
        root.addView(progress,LinearLayout.LayoutParams(-1,dp(3)))
        val notice = MaterialCardView(this).apply { radius = dp(16).toFloat(); cardElevation = 0f; strokeWidth = 0; setCardBackgroundColor(MailTheme.fill); visibility = View.GONE }
        val noticeRow = LinearLayout(this).apply { gravity = Gravity.CENTER_VERTICAL }
        statusView = text("",13f,MailTheme.muted)
        noticeRow.addView(statusView,LinearLayout.LayoutParams(0,-2,1f))
        if (name == "inbox") noticeRow.addView(iconButton(R.drawable.ic_close,"关闭提示") { }.apply {
            setOnClickListener { dismissedInboxStatus = statusView.text.toString(); statusContainer.visibility = View.GONE }
        },LinearLayout.LayoutParams(dp(48),dp(48)))
        notice.addView(noticeRow); statusContainer = notice
        root.addView(notice,LinearLayout.LayoutParams(-1,-2).apply { setMargins(dp(16),dp(4),dp(16),dp(8)) })
        content = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        root.addView(content,LinearLayout.LayoutParams(-1,0,1f))
        if (name == "inbox") {
            val navigation = DrawerLayout(this).apply { setScrimColor(0x66001b3f); addView(root,DrawerLayout.LayoutParams(-1,-1)) }
            drawer = navigation
            setContentView(navigation)
        } else setContentView(root)
        root.requestApplyInsets()
    }
    private fun status(value: String) { if (::statusView.isInitialized) { statusView.text = value; statusContainer.visibility = if (value.isBlank() || (route == "inbox" && value == dismissedInboxStatus)) View.GONE else View.VISIBLE } }
    private fun iconButton(icon: Int, label: String, action: () -> Unit) = ImageButton(this).apply {
        setImageResource(icon); contentDescription = label; imageTintList = ColorStateList.valueOf(MailTheme.muted); setPadding(dp(12),dp(12),dp(12),dp(12))
        val appearance = obtainStyledAttributes(intArrayOf(android.R.attr.selectableItemBackgroundBorderless))
        background = appearance.getDrawable(0); appearance.recycle()
        setOnClickListener { if (!busy) action() }
    }
    private fun navEntry(label: String, icon: Int, selected: Boolean = false, action: () -> Unit) = MaterialButton(this).apply {
        text = label; isAllCaps = false; gravity = Gravity.START or Gravity.CENTER_VERTICAL; minHeight = dp(56); cornerRadius = dp(28)
        insetTop = 0; insetBottom = 0; strokeWidth = 0; setPadding(dp(16),0,dp(16),0)
        backgroundTintList = ColorStateList.valueOf(if (selected) MailTheme.selected else android.graphics.Color.TRANSPARENT)
        setTextColor(if (selected) MailTheme.onPrimaryContainer else MailTheme.text); this.icon = AppCompatResources.getDrawable(this@MainActivity,icon); iconTint = ColorStateList.valueOf(if (selected) MailTheme.onPrimaryContainer else MailTheme.muted)
        contentDescription = label; setOnClickListener { if (busy && cancellableSync) cancelSync(); if (busy) toast("正在处理邮件，请稍候") else action() }
    }
    private fun buildDrawer(): View {
        val panel = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL; setPadding(dp(12),dp(16),dp(12),dp(16))
            background = GradientDrawable().apply { setColor(MailTheme.recessed); cornerRadii = floatArrayOf(0f,0f,dp(28).toFloat(),dp(28).toFloat(),dp(28).toFloat(),dp(28).toFloat(),0f,0f) }
        }
        if (android.os.Build.VERSION.SDK_INT >= 30) panel.setOnApplyWindowInsetsListener { view,insets ->
            val bars = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout())
            view.setPadding(dp(12)+bars.left,dp(16)+bars.top,dp(12),dp(16)+bars.bottom); insets
        }
        panel.addView(text("简信",28f).apply { setTypeface(typeface,Typeface.BOLD); setPadding(dp(16),dp(12),dp(16),dp(20)) })
        panel.addView(navEntry("全部邮件",R.drawable.ic_mail,unified) { openUnifiedInbox() })
        val scroll = ScrollView(this).apply { isFillViewport = false }
        val accounts = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        scroll.addView(accounts); panel.addView(scroll,LinearLayout.LayoutParams(-1,0,1f))
        accounts.addView(text("邮箱",12f,MailTheme.muted).apply { setPadding(dp(16),dp(24),dp(16),dp(8)) })
        if (repository.accounts.isEmpty()) accounts.addView(text("添加邮箱后会显示在这里",14f,MailTheme.muted))
        repository.accounts.forEach { account ->
            val heading = LinearLayout(this).apply { gravity = Gravity.CENTER_VERTICAL; minimumHeight = dp(56); setPadding(dp(16),dp(12),dp(16),0) }
            heading.addView(providerLogo(account,28),LinearLayout.LayoutParams(dp(28),dp(28)).apply { marginEnd = dp(12) })
            val labels = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
            labels.addView(text(account.title,14f).apply { maxLines = 1; ellipsize = TextUtils.TruncateAt.END; setTypeface(typeface,Typeface.BOLD); setPadding(0,0,0,dp(4)) })
            if (account.title != account.email) labels.addView(text(account.email,12f,MailTheme.muted).apply { maxLines = 1; ellipsize = TextUtils.TruncateAt.END; setPadding(0,0,0,dp(4)) })
            heading.addView(labels,LinearLayout.LayoutParams(0,-2,1f))
            heading.contentDescription = "${account.title} 收件箱"
            heading.isFocusable = true
            heading.setOnClickListener { if (cancellableSync) cancelSync(); if (!busy) openAccountInbox(account) }
            val problem = mailboxProblem(account)
            val more = iconButton(R.drawable.ic_more,"${account.title} 更多邮箱操作") { }
            more.imageTintList = ColorStateList.valueOf(if (account.needsAuthorization) MailTheme.warning else if (problem.isNotBlank()) MailTheme.danger else MailTheme.muted)
            if (problem.isNotBlank()) more.contentDescription = "${account.title} 更多邮箱操作，有待处理的邮箱状态"
            more.setOnClickListener { showDrawerMailboxMenu(account,more) }
            heading.addView(more,LinearLayout.LayoutParams(dp(48),dp(48)))
            accounts.addView(heading)
            val known = repository.cachedFolders(account.id)
            val inboxPath = repository.inboxPath(account.id)
            val inbox = known.firstOrNull { it.path == inboxPath } ?: Folder(inboxPath,"Inbox","inbox")
            val visible = (listOf(inbox) + known).distinctBy { it.path }
            visible.forEach { entry ->
                val label = (if (entry.role == "inbox") "Inbox" else if (entry.role == "custom") entry.name else entry.title) +
                    (entry.total?.let { " · $it" } ?: "")
                val icon = if (entry.role == "inbox") R.drawable.ic_inbox else if (entry.role == "drafts") R.drawable.ic_compose else R.drawable.ic_mail
                accounts.addView(navEntry(label,icon,!unified && accountId == account.id && folder == entry.path) { openAccountFolder(account,entry.path) }.apply { contentDescription = "${account.title} $label" })
            }
        }
        panel.addView(View(this).apply { setBackgroundColor(MailTheme.line) },LinearLayout.LayoutParams(-1,dp(1)).apply { setMargins(dp(16),dp(12),dp(16),dp(8)) })
        panel.addView(navEntry("草稿",R.drawable.ic_compose) { showDrafts() })
        panel.addView(navEntry("发件记录",R.drawable.ic_mail) { showOutbox() })
        return panel
    }
    private fun mailboxProblem(account: MailAccount): String = if (account.needsAuthorization) {
        if (account.oauthSettings != null) "待授权，请点击授权登录" else "缺少访问令牌，请更新邮箱凭据"
    }
        else mailboxErrors[account.id] ?: inboxErrors[account.id].orEmpty()
    private fun showDrawerMailboxMenu(account: MailAccount, anchor: View) {
        val current = repository.accounts.firstOrNull { it.id == account.id } ?: return
        val popup = PopupMenu(this,anchor,Gravity.END)
        fun item(label: String, icon: Int, run: () -> Unit): android.view.MenuItem = popup.menu.add(label).apply {
            this.icon = AppCompatResources.getDrawable(this@MainActivity,icon)?.mutate()?.apply { setTint(MailTheme.muted) }
            setOnMenuItemClickListener {
                if (cancellableSync) cancelSync()
                if (busy) toast("正在处理邮箱操作，请稍候") else run()
                true
            }
        }
        item("刷新文件夹",R.drawable.ic_refresh) { loadAccountFolders(current) }.isEnabled = !current.needsAuthorization
        val problem = mailboxProblem(current)
        if (problem.isNotBlank()) popup.menu.add("邮箱状态：$problem").apply {
            icon = AppCompatResources.getDrawable(this@MainActivity,R.drawable.ic_error)?.mutate()?.apply { setTint(if (current.needsAuthorization) MailTheme.warning else MailTheme.danger) }
            setOnMenuItemClickListener {
                MaterialAlertDialogBuilder(this@MainActivity).setTitle("${current.title} 邮箱状态").setMessage(problem).setPositiveButton("关闭",null).show()
                true
            }
        }
        if (current.oauth2 && current.oauthSettings != null) item(if (current.needsAuthorization) "授权登录" else "重新授权",R.drawable.ic_lock) { showOAuthLogin(current) }
        else if (current.oauth2) item("更新访问令牌",R.drawable.ic_lock) { showAccountEditor(current.copy()) }
        else if (problem.isNotBlank()) item("更新密码 / 授权码",R.drawable.ic_lock) { showAccountEditor(current.copy()) }
        item("编辑邮箱",R.drawable.ic_settings) { showAccountEditor(current.copy()) }
        popup.setForceShowIcon(true); popup.show()
    }
    private fun mailboxFailure(id: String, error: Exception) {
        val message = MailRepository.friendly(error)
        mailboxErrors[id] = message
        inboxErrors = inboxErrors + (id to message)
        status(message); refreshDrawer()
    }
    private fun clearMailboxFailure(id: String) {
        mailboxErrors.remove(id)
        inboxErrors = inboxErrors - id
        if (route == "inbox") {
            if (unified) status(inboxErrors.entries.joinToString("\n") { (account,error) -> "${repository.accounts.firstOrNull { it.id == account }?.title ?: "邮箱"}：$error" })
            else status("")
        }
    }
    private fun providerIcon(account: MailAccount): Int = when (MailProviders.key(account)) {
        "gmail" -> R.drawable.mail_provider_gmail; "outlook" -> R.drawable.mail_provider_outlook
        "qq" -> R.drawable.mail_provider_qq; "foxmail" -> R.drawable.mail_provider_foxmail
        "netease163" -> R.drawable.mail_provider_netease163; "netease126" -> R.drawable.mail_provider_netease126
        "yeah" -> R.drawable.mail_provider_yeah; "aliyun" -> R.drawable.mail_provider_aliyun
        "sina" -> R.drawable.mail_provider_sina; "mail139" -> R.drawable.mail_provider_mail139
        "icloud" -> R.drawable.mail_provider_icloud; "zoho" -> R.drawable.mail_provider_zoho
        "yahoo" -> R.drawable.mail_provider_yahoo; "fastmail" -> R.drawable.mail_provider_fastmail
        "proton" -> R.drawable.mail_provider_proton; "tencent" -> R.drawable.mail_provider_tencent
        else -> R.drawable.ic_mail
    }
    private fun providerLogo(account: MailAccount, size: Int) = ImageView(this).apply {
        setImageResource(providerIcon(account)); contentDescription = "邮箱提供商 ${MailProviders.key(account).ifEmpty { "自定义" }}"
        scaleType = ImageView.ScaleType.FIT_CENTER; minimumWidth = dp(size); minimumHeight = dp(size)
    }
    private fun openMailboxDrawer() {
        val navigation = drawer ?: return
        if (navigation.childCount < 2) navigation.addView(buildDrawer(),DrawerLayout.LayoutParams(dp(320).coerceAtMost(resources.displayMetrics.widthPixels - dp(56)),-1,Gravity.START))
        navigation.openDrawer(GravityCompat.START)
    }
    private fun refreshDrawer() {
        val navigation = drawer ?: return
        if (navigation.childCount < 2) return
        val open = navigation.isDrawerOpen(GravityCompat.START)
        fun scroll(panel: View?): ScrollView? = (panel as? ViewGroup)?.let { group -> (0 until group.childCount).map { group.getChildAt(it) }.filterIsInstance<ScrollView>().firstOrNull() }
        val scrollPosition = scroll(navigation.getChildAt(1))?.scrollY ?: 0
        if (navigation.childCount > 1) navigation.removeViewAt(1)
        val panel = buildDrawer()
        navigation.addView(panel,DrawerLayout.LayoutParams(dp(320).coerceAtMost(resources.displayMetrics.widthPixels - dp(56)),-1,Gravity.START))
        if (open) navigation.openDrawer(GravityCompat.START)
        scroll(panel)?.let { view -> view.post { view.scrollTo(0,scrollPosition) } }
    }
    private fun loadAccountFolders(account: MailAccount) {
        io({ repository.folders(account.id) },failure = { error -> mailboxFailure(account.id,error) },cancellable = true) { found ->
            clearMailboxFailure(account.id)
            if (account.id == accountId) folders = found
            refreshDrawer(); drawer?.openDrawer(GravityCompat.START)
        }
    }
    private fun openUnifiedInbox() {
        unified = true; query = ""; unread = false; messages = emptyList(); more = false; offset = 0; unifiedInbox = null; inboxErrors = emptyMap()
        showInbox(); reload()
    }
    private fun openAccountInbox(account: MailAccount) {
        openAccountFolder(account,repository.inboxPath(account.id))
    }
    private fun openAccountFolder(account: MailAccount, path: String) {
        unified = false; accountId = account.id; query = ""; unread = false; folders = repository.cachedFolders(accountId); folder = path; messages = emptyList(); more = false; offset = 0
        if (account.needsAuthorization) showOAuthLogin(account) else { showInbox(); refreshFolders() }
    }
    private fun composeNew() {
        if (repository.accounts.isEmpty()) { showAccounts(); return }
        if (unified && repository.accounts.size > 1) MaterialAlertDialogBuilder(this).setTitle("选择发件邮箱").setItems(repository.accounts.map { it.title }.toTypedArray()) { _,index ->
            showComposer(Draft(accountId = repository.accounts[index].id))
        }.show()
        else showComposer(Draft(accountId = repository.accounts.firstOrNull { it.id == accountId }?.id ?: repository.accounts.first().id))
    }
    private fun cancelSync() {
        if (!cancellableSync) return
        operation++; generation++; busy = false; cancellableSync = false; progress.visibility = View.GONE
        activeRead?.cancel(); activeRead = null
        activeWork?.cancel(true); activeWork = null
    }
    private fun <T> io(work: () -> T, failure: ((Exception) -> Unit)? = null, cancellable: Boolean = false, complete: (T) -> Unit) {
        if (busy) { toast("正在处理邮箱操作，请稍候"); return }
        busy = true; cancellableSync = cancellable; val task = ++operation; if (::progress.isInitialized) progress.visibility = View.VISIBLE
        val run = generation
        val token = if (cancellable) MailCancellation() else null
        activeRead = token
        activeWork = worker.submit {
            val result = runCatching { MailCancellation.run(token,work) }
            handler.post {
                if (isDestroyed || task != operation) return@post
                activeRead = null; activeWork = null
                busy = false; cancellableSync = false; if (::progress.isInitialized) progress.visibility = View.GONE
                if (run != generation) { result.exceptionOrNull()?.let { toast(MailRepository.friendly(it as? Exception ?: Exception(it))) }; return@post }
                result.fold(onSuccess = complete, onFailure = { cause ->
                    val error = cause as? Exception ?: Exception(cause)
                    // Keep diagnostic locations without exception messages, protocol data or credentials.
                    android.util.Log.e("Jianxin",generateSequence(cause) { it.cause }.take(4).joinToString("\n") { failure ->
                        failure.javaClass.name + "\n" + failure.stackTrace.take(12).joinToString("\n") { "  at $it" }
                    })
                    if (failure != null) failure(error) else if (error is OAuthLoginRequired) {
                        repository.accounts.firstOrNull { it.id == accountId }?.let { showOAuthLogin(it) }
                    } else { status(MailRepository.friendly(error)); toast("操作失败，可以重试") }
                })
            }
        }
    }
    private fun showInbox() {
        if (route == "reader" && restoreInbox()) return
        page(if (unified) "全部邮件" else folders.firstOrNull { it.path == folder }?.title ?: "Inbox","inbox",false)
        composeDraft = null
        val account = repository.accounts.firstOrNull { it.id == accountId }
        toolbar.subtitle = if (unified) "所有邮箱的收件箱" else account?.email ?: ""
        toolbar.menu.add("刷新").apply { setIcon(R.drawable.ic_refresh); setShowAsAction(android.view.MenuItem.SHOW_AS_ACTION_IF_ROOM); setOnMenuItemClickListener { refreshFolders(); true } }
        toolbar.menu.add("管理邮箱").setOnMenuItemClickListener { if (cancellableSync) cancelSync(); if (!busy) showAccounts(); true }
        toolbar.menu.add("设置").setOnMenuItemClickListener { if (cancellableSync) cancelSync(); if (!busy) showSettings(); true }
        if (!unified) toolbar.menu.add("其他文件夹").setOnMenuItemClickListener { if (!busy) chooseFolder(); true }
        val searchCard = MaterialCardView(this).apply { radius = dp(28).toFloat(); cardElevation = 0f; strokeWidth = 0; setCardBackgroundColor(MailTheme.surfaceContainer) }
        val searchRow = LinearLayout(this).apply { gravity = Gravity.CENTER_VERTICAL; setPadding(dp(4),0,dp(4),0) }
        val search = EditText(this).apply { hint = if (unified) "搜索所有邮箱" else "搜索邮件"; contentDescription = "搜索邮件"; setText(query); isSingleLine = true; textSize = 16f; setTextColor(MailTheme.text); setHintTextColor(MailTheme.muted); background = null; imeOptions = EditorInfo.IME_ACTION_SEARCH; minHeight = dp(56); setPadding(dp(8),0,dp(8),0) }
        search.setOnEditorActionListener { _,action,_ -> if (action == EditorInfo.IME_ACTION_SEARCH) { if (cancellableSync) cancelSync(); if (!busy) { query = search.text.toString(); reload() }; true } else false }
        searchRow.addView(iconButton(R.drawable.ic_search,"搜索") { query = search.text.toString(); reload() },LinearLayout.LayoutParams(dp(48),dp(48)))
        searchRow.addView(search,LinearLayout.LayoutParams(0,-2,1f))
        searchRow.addView(iconButton(R.drawable.ic_close,"清除搜索") { search.setText(""); query = ""; reload() },LinearLayout.LayoutParams(dp(48),dp(48)))
        searchCard.addView(searchRow); content.addView(searchCard,LinearLayout.LayoutParams(-1,dp(56)).apply { setMargins(dp(16),dp(8),dp(16),dp(4)) })
        val chips = ChipGroup(this).apply { isSingleSelection = true; isSelectionRequired = true; setPadding(dp(16),dp(4),dp(16),dp(4)) }
        listOf("全部" to false,"未读" to true).forEach { (label,filter) ->
            chips.addView(Chip(this).apply { id = View.generateViewId(); text = label; isCheckable = true; isChecked = unread == filter; setOnCheckedChangeListener { _,checked -> if (checked && unread != filter) { if (cancellableSync) cancelSync(); if (!busy) { unread = filter; reload() } } } })
        }
        content.addView(chips)
        val frame = FrameLayout(this)
        val list = ListView(this).apply { divider = null; setBackgroundColor(MailTheme.canvas); clipToPadding = false; setPadding(0,0,0,dp(96)) }
        loadMoreButton = button("加载更多邮件") { if (more) reload(true) }.apply { visibility = if (more) View.VISIBLE else View.GONE }
        val footer = FrameLayout(this).apply { addView(loadMoreButton,FrameLayout.LayoutParams(-1,-2).apply { setMargins(dp(24),dp(8),dp(24),dp(8)) }) }
        list.addFooterView(footer,null,false)
        adapter = MailAdapter(); list.adapter = adapter
        list.setOnItemClickListener { _,_,position,_ -> if (cancellableSync) cancelSync(); if (!busy) threads.getOrNull(position)?.let(::showThread) }
        frame.addView(list,FrameLayout.LayoutParams(-1,-1))
        val empty = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; gravity = Gravity.CENTER; setPadding(dp(32),dp(24),dp(32),dp(80)) }
        empty.addView(ImageView(this).apply { setImageResource(R.drawable.ic_mail); imageTintList = ColorStateList.valueOf(MailTheme.primary) },LinearLayout.LayoutParams(dp(56),dp(56)))
        empty.addView(text("还没有邮件",22f).apply { gravity = Gravity.CENTER })
        empty.addView(text(if (repository.accounts.isEmpty()) "添加或导入邮箱，在这里查看全部收件箱。" else "点击刷新获取邮件，也可从抽屉选择邮箱。",14f,MailTheme.muted).apply { gravity = Gravity.CENTER })
        empty.addView(button(if (repository.accounts.isEmpty()) "添加或导入邮箱" else "刷新邮件") { if (repository.accounts.isEmpty()) showAccounts() else refreshFolders() })
        frame.addView(empty,FrameLayout.LayoutParams(-1,-1)); list.emptyView = empty
        frame.addView(ExtendedFloatingActionButton(this).apply {
            text = "写邮件"; icon = AppCompatResources.getDrawable(this@MainActivity,R.drawable.ic_compose); contentDescription = "写邮件"
            setOnClickListener { if (cancellableSync) cancelSync(); if (!busy) composeNew() }
        },FrameLayout.LayoutParams(-2,dp(56),Gravity.BOTTOM or Gravity.END).apply { marginStart = dp(16); marginEnd = dp(88); bottomMargin = dp(20) })
        frame.addView(FloatingActionButton(this).apply {
            setImageResource(R.drawable.ic_add); contentDescription = "添加邮箱"; size = FloatingActionButton.SIZE_NORMAL
            backgroundTintList = ColorStateList.valueOf(MailTheme.selected); imageTintList = ColorStateList.valueOf(MailTheme.onPrimaryContainer)
            setOnClickListener { if (cancellableSync) cancelSync(); if (!busy) showProviderPicker() }
        },FrameLayout.LayoutParams(dp(56),dp(56),Gravity.BOTTOM or Gravity.END).apply { marginEnd = dp(20); bottomMargin = dp(20) })
        content.addView(frame,LinearLayout.LayoutParams(-1,0,1f))
        try {
            messages = if (unified) UnifiedInbox.sort(repository.accounts.flatMap { runCatching { repository.cachedInbox(it.id,query,unread) }.getOrDefault(emptyList()) })
                else if (accountId.isNotEmpty()) repository.cachedList(repository.listKey(accountId,folder,query,unread)) else emptyList()
            refreshMailRows()
        } catch (e: Exception) { status(MailRepository.friendly(e)) }
        if (!unified && account?.needsAuthorization == true) status("此邮箱待授权，请在抽屉中点击邮箱登录")
        if (unified && inboxErrors.isNotEmpty()) status(inboxErrors.entries.joinToString("\n") { (id,error) -> "${repository.accounts.firstOrNull { it.id == id }?.title ?: "邮箱"}：$error" })
        statusView.setOnClickListener { openMailboxDrawer() }
    }
    private fun refreshFolders() {
        if (busy) return
        dismissedInboxStatus = null
        if (unified) { reload(); return }
        if (accountId.isEmpty()) return
        repository.accounts.firstOrNull { it.id == accountId && it.needsAuthorization }?.let { showOAuthLogin(it); return }
        try { folders = repository.cachedFolders(accountId) } catch (_: Exception) { }
        val id = accountId
        io({ repository.folders(id) },failure = { error -> mailboxFailure(id,error) },cancellable = true) { found -> clearMailboxFailure(id); folders = found; if (found.none { it.path == folder }) folder = found.firstOrNull { it.role == "inbox" }?.path ?: "INBOX"; refreshDrawer(); if (route == "inbox") reload() }
    }
    private fun reload(next: Boolean = false) {
        if (busy) return
        if (!next) dismissedInboxStatus = null
        if (unified) { reloadUnified(next); return }
        if (accountId.isEmpty()) return
        repository.accounts.firstOrNull { it.id == accountId && it.needsAuthorization }?.let { showOAuthLogin(it); return }
        val id = accountId; val path = folder; val search = query; val onlyUnread = unread; val start = if (next) offset else 0
        val key = repository.listKey(id,path,search,onlyUnread)
        if (!next) { offset = 0; more = false; try { messages = repository.cachedList(key); refreshMailRows() } catch (_: Exception) { } }
        status("正在连接邮箱…")
        io({ repository.page(id,path,search,onlyUnread,start) },failure = { error -> mailboxFailure(id,error) },cancellable = true) { page ->
            clearMailboxFailure(id); refreshDrawer()
            messages = if (next) (messages + page.first).distinctBy { it.id } else page.first
            offset = start + 30; more = page.second
            try { repository.cacheList(id,key,messages) } catch (_: Exception) { toast("本地缓存保存失败") }
            refreshMailRows(); loadMoreButton?.visibility = if (more) View.VISIBLE else View.GONE
            toolbar.subtitle = repository.account(id).title + " · ${messages.size} 封邮件"; status(if (messages.isEmpty()) "没有符合条件的邮件" else "")
        }
    }
    private fun reloadUnified(next: Boolean = false) {
        if (repository.accounts.isEmpty()) { status(""); return }
        val search = query; val onlyUnread = unread
        if (!next || unifiedInbox == null) unifiedInbox = UnifiedInbox(repository.accounts.map { it.id }) { repository.cachedInbox(it,search,onlyUnread) }
        val pager = unifiedInbox!!
        val scene = generation
        io({ pager.next(cancelled = { generation != scene }) { id,start -> repository.inboxBatch(id,search,onlyUnread,start) { generation != scene } } },cancellable = true) { result ->
            messages = result.messages.filter { !onlyUnread || !it.isRead }; more = result.hasMore; inboxErrors = result.errors
            repository.accounts.forEach { account ->
                val error = result.errors[account.id]
                if (error == null) mailboxErrors.remove(account.id) else mailboxErrors[account.id] = error
            }
            refreshMailRows(); refreshDrawer(); loadMoreButton?.visibility = if (more) View.VISIBLE else View.GONE
            toolbar.subtitle = "${repository.accounts.size} 个邮箱 · ${messages.size} 封邮件"
            status(result.errors.entries.joinToString("\n") { (id,error) -> "${repository.accounts.firstOrNull { it.id == id }?.title ?: "邮箱"}：$error" })
        }
    }
    private fun chooseFolder() {
        if (folders.isEmpty()) { toast("先刷新邮箱文件夹"); return }
        MaterialAlertDialogBuilder(this).setTitle("选择文件夹").setItems(folders.map { it.title }.toTypedArray()) { _,i ->
            if (busy) { toast("等待当前操作完成"); return@setItems }; folder = folders[i].path; messages = emptyList(); showInbox(); reload()
        }.show()
    }
    private inner class MailAdapter : BaseAdapter() {
        override fun getCount() = threads.size
        override fun getItem(position: Int) = threads[position]
        override fun getItemId(position: Int) = position.toLong()
        override fun getView(position: Int, convertView: View?, parent: ViewGroup): View {
            val thread = threads[position]; val mail = thread.latest
            val holder = convertView?.tag as? MailRow ?: run {
                val view = LinearLayout(this@MainActivity).apply { orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL; descendantFocusability = ViewGroup.FOCUS_BLOCK_DESCENDANTS; setPadding(dp(16),dp(12),dp(12),dp(12)); minimumHeight = dp(112) }
                val avatar = TextView(this@MainActivity).apply { gravity = Gravity.CENTER; textSize = 20f; setTypeface(typeface,Typeface.NORMAL); importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO }
                view.addView(avatar,LinearLayout.LayoutParams(dp(44),dp(44)).apply { marginEnd = dp(16) })
                val body = LinearLayout(this@MainActivity).apply { orientation = LinearLayout.VERTICAL }
                val sender = TextView(this@MainActivity).apply { textSize = 16f; setTextColor(MailTheme.text); maxLines = 1; ellipsize = TextUtils.TruncateAt.END }
                val subject = TextView(this@MainActivity).apply { textSize = 14f; setTextColor(MailTheme.text); maxLines = 2; ellipsize = TextUtils.TruncateAt.END; setPadding(0,dp(4),0,dp(8)) }
                val source = TextView(this@MainActivity).apply { textSize = 11f; maxLines = 1; ellipsize = TextUtils.TruncateAt.END; setPadding(dp(8),dp(2),dp(8),dp(2)) }
                body.addView(sender); body.addView(subject); body.addView(source,LinearLayout.LayoutParams(-2,-2))
                view.addView(body,LinearLayout.LayoutParams(0,-2,1f))
                val trailing = LinearLayout(this@MainActivity).apply { orientation = LinearLayout.VERTICAL; gravity = Gravity.END or Gravity.CENTER_VERTICAL; setPadding(dp(8),0,0,0) }
                val date = TextView(this@MainActivity).apply { textSize = 11f; setTextColor(MailTheme.muted); gravity = Gravity.END; maxLines = 2; maxWidth = dp(100); ellipsize = TextUtils.TruncateAt.END }
                val star = iconButton(R.drawable.ic_star_outline,"星标") { }
                trailing.addView(date); trailing.addView(star,LinearLayout.LayoutParams(dp(48),dp(48)))
                view.addView(trailing,LinearLayout.LayoutParams(-2,-2))
                MailRow(view,avatar,sender,subject,source,date,star).also { view.tag = it }
            }
            val source = repository.accounts.firstOrNull { it.id == mail.accountId }
            val tints = listOf(MailTheme.blueTint to MailTheme.blueText,MailTheme.greenTint to MailTheme.greenText,MailTheme.violetTint to MailTheme.violetText)
            val tint = tints[Math.floorMod(mail.accountId.hashCode(),tints.size)]
            holder.avatar.apply { text = mail.sender.take(1).uppercase(java.util.Locale.ROOT); setTextColor(tint.second); background = GradientDrawable().apply { shape = GradientDrawable.OVAL; setColor(tint.first) } }
            holder.sender.apply { text = mail.sender + if (thread.messages.size > 1) " · ${thread.messages.size}" else ""; setTypeface(null,if (thread.unread) Typeface.BOLD else Typeface.NORMAL) }
            holder.subject.apply { text = mail.title; setTypeface(null,if (thread.unread) Typeface.BOLD else Typeface.NORMAL) }
            holder.source.apply {
                text = source?.title ?: "邮箱"; setTextColor(tint.second); background = GradientDrawable().apply { cornerRadius = dp(6).toFloat(); setColor(tint.first) }
                val logo = source?.let { AppCompatResources.getDrawable(this@MainActivity,providerIcon(it)) }?.apply { setBounds(0,0,dp(16),dp(16)) }
                setCompoundDrawables(logo,null,null,null); compoundDrawablePadding = dp(4)
            }
            holder.date.text = MailTime.format(mail.date,timeStyle,compact = true)
            holder.star.apply {
                setImageResource(if (mail.isStarred) R.drawable.ic_star else R.drawable.ic_star_outline); imageTintList = ColorStateList.valueOf(if (mail.isStarred) MailTheme.warning else MailTheme.muted)
                contentDescription = if (mail.isStarred) "取消星标" else "星标"
                isFocusable = false; setOnClickListener { if (cancellableSync) cancelSync(); if (!busy) io({ repository.flag(mail,star = !mail.isStarred) }) { updateCachedFlags(mail); refreshMailRows() } }
            }
            holder.view.contentDescription = "${if (thread.unread) "未读" else "已读"}，${mail.sender}，${mail.title}，${thread.messages.size} 封邮件"
            return holder.view
        }
    }
    private data class MailRow(val view: LinearLayout, val avatar: TextView, val sender: TextView, val subject: TextView, val source: TextView, val date: TextView, val star: ImageButton)
    private data class InboxPage(val root: LinearLayout, val content: LinearLayout, val toolbar: MaterialToolbar,
        val progress: ProgressBar, val status: TextView, val notice: View, val drawer: DrawerLayout?,
        val more: MaterialButton?, val adapter: MailAdapter?)
    private fun restoreInbox(): Boolean {
        val saved = retainedInbox ?: return false
        if (cancellableSync) cancelSync()
        generation++; route = "inbox"; retainedInbox = null
        root = saved.root; content = saved.content; toolbar = saved.toolbar; progress = saved.progress
        statusView = saved.status; statusContainer = saved.notice; drawer = saved.drawer; loadMoreButton = saved.more; adapter = saved.adapter
        composeDraft = null; currentMessage = null; conversationMessages = emptyList(); readerDetails.clear(); expandedMessages.clear(); offlineMessages.clear()
        progress.visibility = if (busy) View.VISIBLE else View.GONE
        setContentView(drawer ?: root); root.requestApplyInsets(); refreshMailRows()
        return true
    }
    private fun refreshMailRows() { threads = MailThreads.group(messages); adapter?.notifyDataSetChanged() }
    private fun showThread(thread: MailThread) {
        retainedInbox = InboxPage(root,content,toolbar,progress,statusView,statusContainer,drawer,loadMoreButton,adapter)
        page("邮件会话","reader")
        toolbar.menu.add("刷新").apply { setIcon(R.drawable.ic_refresh); setShowAsAction(android.view.MenuItem.SHOW_AS_ACTION_IF_ROOM); setOnMenuItemClickListener { reloadReader(); true } }
        val moreId = View.generateViewId()
        toolbar.menu.add(0,moreId,1,"更多").apply { setIcon(R.drawable.ic_more); setShowAsAction(android.view.MenuItem.SHOW_AS_ACTION_ALWAYS); setOnMenuItemClickListener { showReaderMenu(toolbar.findViewById<View>(moreId) ?: toolbar); true } }
        val summary = thread.latest
        currentMessage = summary; readerNotice = "正在读取邮件…"; readerDetails.clear(); offlineMessages.clear(); expandedMessages.clear(); expandedMessages.add(summary.id)
        conversationMessages = thread.messages
        renderThread()
        readThreadMessage(summary) { loadConversation(summary,thread.messages) }
    }
    private fun loadConversation(summary: Message, seed: List<Message>, forceRefresh: Boolean = false) {
        val scene = generation; readerNotice = "正在加载同一会话的邮件…"; status(readerNotice)
        io({ repository.conversation(summary,seed,forceRefresh) { generation != scene } },failure = {
            readerNotice = "会话加载失败，已读取的邮件可继续查看；点击重新读取重试"; updateReaderStatus()
        },cancellable = true) { found ->
            val changed = conversationMessages.map { it.id to it.date } != found.first.map { it.id to it.date }
            conversationMessages = found.first
            conversationMessages.forEach { header -> if (header.date != 0L) readerDetails[header.id]?.date = header.date }
            readerNotice = if (found.second) "部分文件夹暂时无法读取，点击重新读取重试" else ""
            if (changed) renderThread() else updateReaderStatus()
        }
    }
    private fun readThreadMessage(summary: Message, forceRefresh: Boolean = false, after: (() -> Unit)? = null) {
        val scene = generation
        io({
            val cached = runCatching { repository.cachedDetail(summary) }.getOrNull()
            cached?.takeIf { forceRefresh || !summary.isRead }?.let {
                MailCancellation.checkCurrent()
                handler.post {
                    if (!isDestroyed && route == "reader" && generation == scene) {
                        readerDetails[summary.id] = cached; offlineMessages.add(summary.id); renderThread()
                    }
                }
            }
            repository.detail(summary,forceRefresh,cached)
        },failure = { error ->
            readerNotice = MailRepository.friendly(error); renderThread(); after?.invoke()
        },cancellable = true) { found ->
            val mail = found.first; readerDetails[mail.id] = mail; currentMessage = mail
            if (found.second) offlineMessages.add(mail.id) else offlineMessages.remove(mail.id)
            updateCachedFlags(mail)
            readerNotice = ""; renderThread(); after?.invoke()
        }
    }
    private fun updateReaderStatus() {
        val selected = currentMessage ?: return
        status(listOf(readerNotice,if (selected.id in offlineMessages) "当前正文为本地缓存" else "").filter { it.isNotEmpty() }.joinToString("\n"))
        toolbar.subtitle = "${conversationMessages.size} 封邮件"
    }
    private fun renderThread(focusSelected: Boolean = false) {
        val previousScroll = (0 until content.childCount).map { content.getChildAt(it) }.filterIsInstance<ScrollView>().firstOrNull()
        val scrollPosition = previousScroll?.scrollY ?: 0
        content.removeAllViews()
        val selected = currentMessage ?: return
        val detail = readerDetails[selected.id]
        updateReaderStatus()
        val scroll = ScrollView(this); val reading = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(16),0,dp(16),dp(16)) }
        scroll.addView(reading); content.addView(scroll,LinearLayout.LayoutParams(-1,0,1f))
        reading.addView(text(MailThreads.subject(selected.subject).ifEmpty { selected.title },22f).apply { setTypeface(typeface,Typeface.BOLD); setTextIsSelectable(true); setPadding(0,dp(16),0,dp(12)) })
        repository.accounts.firstOrNull { it.id == selected.accountId }?.let { account ->
            val source = LinearLayout(this).apply { gravity = Gravity.CENTER_VERTICAL; setPadding(0,0,0,dp(12)) }
            source.addView(providerLogo(account,20),LinearLayout.LayoutParams(dp(20),dp(20)))
            source.addView(text(account.title,13f,MailTheme.muted).apply { setPadding(dp(8),0,0,0) }); reading.addView(source)
        }
        var activeCard: View? = null
        conversationMessages.forEach { summary ->
            val mail = readerDetails[summary.id] ?: summary
            val expanded = mail.id in expandedMessages
            val card = MaterialCardView(this).apply { radius = dp(20).toFloat(); cardElevation = 0f; strokeWidth = dp(1); strokeColor = MailTheme.line; setCardBackgroundColor(MailTheme.canvas) }
            val section = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(4),dp(8),dp(4),dp(8)) }
            val header = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; minimumHeight = dp(64); contentDescription = "${if (expanded) "收起" else "展开"}邮件 ${mail.sender}" }
            val senderRow = LinearLayout(this).apply { gravity = Gravity.CENTER_VERTICAL; setPadding(dp(16),dp(8),dp(16),0) }
            repository.accounts.firstOrNull { it.id == mail.accountId }?.let { account -> senderRow.addView(providerLogo(account,24),LinearLayout.LayoutParams(dp(24),dp(24))) }
            senderRow.addView(text(mail.sender,16f).apply { setTypeface(typeface,if (mail.isRead) Typeface.NORMAL else Typeface.BOLD); setPadding(dp(12),0,dp(8),0) },LinearLayout.LayoutParams(0,-2,1f))
            senderRow.addView(ImageView(this).apply { setImageResource(R.drawable.ic_back); rotation = if (expanded) 90f else -90f; imageTintList = ColorStateList.valueOf(MailTheme.muted); importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO },LinearLayout.LayoutParams(dp(20),dp(20)))
            header.addView(senderRow); header.addView(text(MailTime.format(mail.date,timeStyle),12f,MailTheme.muted))
            header.setOnClickListener {
                if (cancellableSync) cancelSync()
                if (!busy) {
                    if (!expandedMessages.add(mail.id)) expandedMessages.remove(mail.id) else currentMessage = mail
                    renderThread(focusSelected = mail.id in expandedMessages)
                    if (mail.id in expandedMessages && (mail.id !in readerDetails || mail.id in offlineMessages)) { readerNotice = "正在读取邮件…"; status(readerNotice); readThreadMessage(mail) }
                }
            }
            section.addView(header)
            if (expanded) {
                section.addView(text("${mail.from.joinToString(", ") { it.email }}\n收件人：${mail.to.joinToString(", ") { it.email }}" + if (mail.cc.isNotEmpty()) "\n抄送：${mail.cc.joinToString(", ") { it.email }}" else "",13f,MailTheme.muted).apply { setTextIsSelectable(true) })
                section.addView(text(mail.body?.ifEmpty { "（邮件没有可显示的正文）" } ?: "点击读取正文").apply { setTextIsSelectable(true); setLineSpacing(dp(4).toFloat(),1f) })
                if (mail.body == null) section.addView(button("读取正文",true) { if (cancellableSync) cancelSync(); currentMessage = mail; readThreadMessage(mail) })
                if (mail.attachments.isNotEmpty()) section.addView(ChipGroup(this).apply {
                    setPadding(dp(12),dp(4),dp(12),dp(4))
                    mail.attachments.forEach { attachment -> addView(Chip(this@MainActivity).apply {
                        text = attachment.filename; maxWidth = resources.displayMetrics.widthPixels - dp(80); ellipsize = TextUtils.TruncateAt.END
                        chipIcon = AppCompatResources.getDrawable(this@MainActivity,R.drawable.ic_attachment); isChipIconVisible = true
                        contentDescription = "保存附件 ${attachment.filename}，${attachment.size / 1024} KB"
                        setOnClickListener { if (!busy) exportAttachment(attachment) }
                    }) }
                })
            }
            card.addView(section); reading.addView(card,LinearLayout.LayoutParams(-1,-2).apply { bottomMargin = dp(12) })
            if (mail.id == selected.id) activeCard = card
        }
        val actions = LinearLayout(this).apply { gravity = Gravity.END or Gravity.CENTER_VERTICAL; setPadding(dp(16),dp(8),dp(16),dp(12)); setBackgroundColor(MailTheme.recessed) }
        actions.addView(iconButton(R.drawable.ic_reply_all,"回复全部") { detail?.let { showComposer(repository.reply(it,true)) } }.apply {
            isEnabled = detail != null; setOnClickListener { if (cancellableSync) cancelSync(); if (!busy) detail?.let { showComposer(repository.reply(it,true)) } }
        },LinearLayout.LayoutParams(dp(48),dp(48)).apply { marginEnd = dp(12) })
        actions.addView(MaterialButton(this).apply {
            text = "回复"; icon = AppCompatResources.getDrawable(this@MainActivity,R.drawable.ic_reply); isAllCaps = false; cornerRadius = dp(24); minHeight = dp(48)
            isEnabled = detail != null; setOnClickListener { if (cancellableSync) cancelSync(); if (!busy) detail?.let { showComposer(repository.reply(it,false)) } }
        },LinearLayout.LayoutParams(-2,-2))
        content.addView(actions)
        scroll.post { if (focusSelected || previousScroll == null) activeCard?.let { scroll.scrollTo(0,it.top) } else scroll.scrollTo(0,scrollPosition) }
    }
    private fun reloadReader() {
        if (cancellableSync) cancelSync()
        if (busy || conversationMessages.isEmpty()) return
        val selected = currentMessage ?: conversationMessages.last()
        readerNotice = "正在读取邮件…"; status(readerNotice)
        readThreadMessage(selected,forceRefresh = true) { loadConversation(selected,conversationMessages,forceRefresh = true) }
    }
    private fun showReaderMenu(anchor: View) {
        val selected = currentMessage ?: return
        val mail = readerDetails[selected.id] ?: selected
        val popup = PopupMenu(this,anchor)
        fun item(label: String, icon: Int, enabled: Boolean = true, action: () -> Unit) {
            popup.menu.add(label).apply {
                this.icon = AppCompatResources.getDrawable(this@MainActivity,icon)?.mutate()?.apply { setTint(MailTheme.muted) }; isEnabled = enabled
                setOnMenuItemClickListener { if (cancellableSync) cancelSync(); if (busy) toast("正在处理邮件，请稍候") else action(); true }
            }
        }
        item("回复全部",R.drawable.ic_reply_all,selected.id in readerDetails) { showComposer(repository.reply(mail,true)) }
        item("转发",R.drawable.ic_forward,selected.id in readerDetails) { showComposer(Draft(accountId = mail.accountId,subject = "Fwd: ${mail.subject}",text = "\n\n—— 转发邮件 ——\n${mail.body ?: ""}",attachments = mail.attachments)) }
        item(if (mail.isStarred) "取消星标" else "星标",if (mail.isStarred) R.drawable.ic_star else R.drawable.ic_star_outline) { io({ repository.flag(mail,star = !mail.isStarred) }) { updateCachedFlags(mail); renderThread() } }
        item(if (mail.isRead) "标为未读" else "标为已读",R.drawable.ic_mail) { io({ repository.flag(mail,read = !mail.isRead) }) { updateCachedFlags(mail); renderThread() } }
        val sourceFolders = repository.cachedFolders(mail.accountId)
        val move = popup.menu.addSubMenu("移动到文件夹")
        sourceFolders.filter { it.path != mail.folder }.forEach { target -> move.add(target.title).setOnMenuItemClickListener { if (cancellableSync) cancelSync(); if (!busy) moveMessage(mail,target.path); true } }
        move.item.isEnabled = move.size() > 0
        item("移到已删除",R.drawable.ic_mail) {
            val trash = sourceFolders.firstOrNull { it.role == "trash" }
            if (trash == null) toast("邮箱没有已删除文件夹，请选择移动目标") else MaterialAlertDialogBuilder(this).setTitle("移到已删除？").setMessage(mail.title)
                .setPositiveButton("移到已删除") { _,_ -> moveMessage(mail,trash.path) }.setNegativeButton("取消",null).show()
        }
        item("重新读取",R.drawable.ic_refresh) { reloadReader() }
        popup.setForceShowIcon(true); popup.show()
    }
    private fun updateCachedFlags(mail: Message) {
        messages = messages.map { if (it.id == mail.id) it.copy(isRead = mail.isRead,isStarred = mail.isStarred,date = mail.date) else it }.filter { !unread || !it.isRead }
        if (unread && mail.isRead) unifiedInbox?.remove(mail) else unifiedInbox?.update(mail)
        cacheSourceList(mail)
        refreshMailRows()
    }
    private fun cacheSourceList(mail: Message, removed: Boolean = false) {
        val flags = mail.headerOnly(); val onlyUnread = unread
        val key = repository.listKey(mail.accountId,mail.folder,query,onlyUnread)
        storageWorker.execute {
            try {
                repository.updateCachedMessage(key,flags,removed,onlyUnread)
            } catch (_: Exception) { }
        }
    }
    private fun moveMessage(mail: Message, target: String) { io({ repository.move(mail,target) }) { messages = messages.filter { it.id != mail.id }; unifiedInbox?.remove(mail); cacheSourceList(mail,true); showInbox(); reload() } }
    private fun exportAttachment(attachment: Attachment) {
        if (attachment.data == null) { toast("附件尚未下载，重新读取邮件后重试"); return }
        attachmentExport = attachment
        startActivityForResult(Intent(Intent.ACTION_CREATE_DOCUMENT).apply { addCategory(Intent.CATEGORY_OPENABLE); type = attachment.mimeType; putExtra(Intent.EXTRA_TITLE,attachment.filename.substringAfterLast('/').substringAfterLast('\\')) },EXPORT)
    }
    private fun showDrafts() {
        page("本地草稿","drafts")
        toolbarAction("写邮件",R.drawable.ic_compose) { composeNew() }
        val list = material.form(content)
        val drafts = repository.drafts.filter { it.delivery == "draft" }.sortedByDescending { it.date }
        if (drafts.isEmpty()) material.empty(list,R.drawable.ic_compose,"还没有草稿","写信时会自动保存到本机，下次可继续编辑。")
        drafts.forEach { draft ->
            val card = material.card(list)
            val more = iconButton(R.drawable.ic_more,"草稿操作") { }
            more.setOnClickListener {
                val popup = PopupMenu(this,more)
                popupItem(popup,"继续编辑",R.drawable.ic_compose) { showComposer(draft.copy()) }
                popupItem(popup,"丢弃草稿",R.drawable.ic_delete,true) { discardDraft(draft) }
                popup.setForceShowIcon(true); popup.show()
            }
            material.listRow(card,draft.title,draft.to.ifBlank { "尚未填写收件人" },material.emblem(R.drawable.ic_compose),more) { if (!busy) showComposer(draft.copy()) }
            card.addView(material.text(MailTime.format(draft.date,timeStyle),12f))
            if (draft.text.isNotBlank()) card.addView(material.text(draft.text.trim(),14f).apply { maxLines = 2; ellipsize = TextUtils.TruncateAt.END; setPadding(0,dp(8),0,0) })
        }
    }
    private fun showOutbox() {
        page("发件记录","outbox")
        val list = material.form(content)
        val jobs = repository.drafts.filter { it.delivery != "draft" }.sortedByDescending { it.date }
        if (jobs.isEmpty()) material.empty(list,R.drawable.ic_send,"还没有发送记录","发送邮件后，可在这里查看投递结果。")
        jobs.forEach { job ->
            val label = mapOf("sent" to "已发送","sending" to "正在发送","uncertain" to "结果未知","failed" to "发送失败")[job.delivery] ?: job.delivery
            val card = material.card(list)
            val more = iconButton(R.drawable.ic_more,"发送记录操作") { }
            more.setOnClickListener {
                val popup = PopupMenu(this,more)
                popupItem(popup,"查看详情",R.drawable.ic_mail) { MaterialAlertDialogBuilder(this).setTitle(job.title).setMessage("$label\n${job.to}\n${MailTime.format(job.date,timeStyle)}" + if (job.error.isNotEmpty()) "\n\n${job.error}" else "").setPositiveButton("关闭",null).show() }
                if (job.delivery in listOf("failed","uncertain")) popupItem(popup,"重试发送",R.drawable.ic_refresh) {
                    MaterialAlertDialogBuilder(this).setTitle("确认重发？").setMessage(if (job.delivery == "uncertain") "服务器可能已收到邮件。先检查已发送文件夹，再决定重发，避免重复发送。" else job.title)
                        .setPositiveButton("确认重发") { _,_ -> io({ repository.send(job,true) }) { toast(it); showOutbox() } }.setNegativeButton("取消",null).show()
                }
                popup.setForceShowIcon(true); popup.show()
            }
            material.listRow(card,job.title,job.to,material.emblem(R.drawable.ic_send),more)
            card.addView(material.text("$label · ${MailTime.format(job.date,timeStyle)}",12f,if (job.delivery == "sent") MailTheme.success else MailTheme.warning))
            if (job.error.isNotEmpty()) card.addView(material.text(job.error,13f,MailTheme.warning).apply { setPadding(0,dp(8),0,0) })
        }
    }
    private fun showSettings() {
        page("设置","settings")
        val form = material.form(content)
        val time = material.card(form,"时间显示")
        time.addView(material.text("邮件列表和阅读页使用此格式，按手机的时区显示。"))
        var selected = if (timeStyle in listOf(MailTime.DEFAULT,MailTime.TWELVE)) timeStyle else "custom"
        val choices = ChipGroup(this).apply { isSingleSelection = true; isSelectionRequired = true; setPadding(dp(8),dp(8),dp(8),dp(8)) }
        time.addView(choices)
        val custom = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; visibility = if (selected == "custom") View.VISIBLE else View.GONE }
        val pattern = input(custom,"自定义格式",if (selected == "custom") timeStyle else "yyyy-MM-dd HH:mm")
        custom.addView(material.text("例如 yyyy-MM-dd HH:mm:ss。HH 为 24 小时，hh 为 12 小时，a 为上午 / 下午。",13f)); time.addView(custom)
        val previewCard = material.card(form,"显示预览")
        val preview = material.text("",20f,MailTheme.text,true); previewCard.addView(preview)
        fun updatePreview() { preview.text = runCatching { "预览：${MailTime.format(System.currentTimeMillis(),if (selected == "custom") pattern.text.toString().trim() else selected)}" }.getOrElse { "预览：格式无效" } }
        listOf("24 小时" to MailTime.DEFAULT,"12 小时" to MailTime.TWELVE,"自定义" to "custom").forEach { (label,style) ->
            choices.addView(Chip(this).apply {
                id = View.generateViewId(); text = label; isCheckable = true; isChecked = selected == style
                setOnCheckedChangeListener { _,checked -> if (checked) { selected = style; custom.visibility = if (style == "custom") View.VISIBLE else View.GONE; updatePreview() } }
            })
        }
        pattern.addTextChangedListener(object : TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) { }
            override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) { updatePreview() }
            override fun afterTextChanged(s: Editable?) { }
        })
        updatePreview()
        footer(action("保存设置",R.drawable.ic_settings,MailUi.Action.Filled) {
            try {
                val style = if (selected == "custom") pattern.text.toString().trim() else selected
                repository.rememberTimeFormat(style); timeStyle = style; toast("时间格式已保存"); showInbox()
            } catch (error: Exception) { status(MailRepository.friendly(error)) }
        })
    }
    private fun showAccounts() {
        page("管理邮箱","accounts")
        toolbar.subtitle = "${repository.accounts.size} 个邮箱"
        val list = material.form(content)
        val imports = material.card(list,"导入邮箱")
        material.listRow(imports,"密文文件","选择文件并在本机解密",material.emblem(R.drawable.ic_file)) { if (!busy) showConfigImport(localFile = true) }
        material.listRow(imports,"加密配置链接","从 HTTPS 链接获取配置",material.emblem(R.drawable.ic_open)) { if (!busy) showConfigImport() }
        if (repository.accounts.isEmpty()) material.empty(list,R.drawable.ic_mail,"添加第一个邮箱","手动添加邮箱，或导入已有的加密配置。")
        repository.accounts.forEach { account ->
            val card = material.card(list)
            val more = iconButton(R.drawable.ic_more,"${account.title} 邮箱操作") { }
            more.setOnClickListener {
                val popup = PopupMenu(this,more)
                popupItem(popup,"编辑邮箱",R.drawable.ic_settings) { showAccountEditor(account.copy()) }
                if (account.oauth2 && account.oauthSettings != null) popupItem(popup,if (account.needsAuthorization) "授权登录" else "重新授权",R.drawable.ic_lock) { showOAuthLogin(account) }
                popupItem(popup,"移除邮箱",R.drawable.ic_delete,true) {
                    MaterialAlertDialogBuilder(this).setTitle("移除邮箱？").setMessage("移除此邮箱的本地缓存、草稿和发送记录，服务器邮件保留。")
                        .setPositiveButton("移除") { _,_ -> io({ repository.removeAccount(account.id) }) { if (accountId == account.id) { accountId = repository.accounts.firstOrNull()?.id ?: ""; messages = emptyList(); folders = emptyList() }; showAccounts() } }.setNegativeButton("取消",null).show()
                }
                popup.setForceShowIcon(true); popup.show()
            }
            material.listRow(card,account.title,if (account.title != account.email) account.email else "",providerLogo(account,40),more) { if (!busy) openAccountInbox(account) }
            if (account.needsAuthorization) card.addView(material.text("待授权 · 点击邮箱登录",12f,MailTheme.primary))
        }
        footer(action("添加邮箱",R.drawable.ic_add,MailUi.Action.Filled) { showProviderPicker() })
    }
    private fun input(parent: LinearLayout, label: String, value: String, type: Int = InputType.TYPE_CLASS_TEXT): EditText {
        return material.field(parent,label,value,type)
    }
    private fun showOAuthLogin(account: MailAccount) {
        page("邮箱授权登录","oauth-login")
        val form = material.form(content)
        val mailbox = material.card(form)
        material.listRow(mailbox,account.title,if (account.title != account.email) account.email else "Microsoft 授权登录",providerLogo(account,40))
        val settings = account.oauthSettings
        if (settings == null || settings.clientId.isBlank()) {
            material.card(form,"先完善授权设置").addView(material.text(if (settings == null) "请在邮箱设置中配置 Microsoft 授权，或填写已有访问令牌。" else "请在邮箱设置中填写 OAuth Client ID（应用 ID），再开始授权。"))
            footer(action("编辑邮箱",R.drawable.ic_settings,MailUi.Action.Filled) { showAccountEditor(account.copy(),forceAdvanced = true) }); return
        }
        material.card(form,"使用 Microsoft 登录").addView(material.text("获取授权码，在 Microsoft 登录页面输入授权码并登录此邮箱。完成后返回简信，自动读取邮件。"))
        moreAction { popup ->
            popupItem(popup,"重新开始授权",R.drawable.ic_refresh) { showOAuthLogin(account) }
            popupItem(popup,"编辑邮箱",R.drawable.ic_settings) { showAccountEditor(account.copy()) }
        }
        val start = action("开始授权登录",R.drawable.ic_lock,MailUi.Action.Filled) { }
        footer(start)
        start.setOnClickListener {
            if (busy) return@setOnClickListener
            start.isEnabled = false
            io({ MicrosoftOAuth.start(settings) },failure = { error -> start.isEnabled = true; status(MailRepository.friendly(error)) }) { session ->
                val codeCard = material.card(form,"Microsoft 授权码")
                val codeRow = LinearLayout(this).apply { gravity = Gravity.CENTER_VERTICAL }
                codeRow.addView(material.text(session.userCode,32f,MailTheme.primary,true).apply { letterSpacing = 0.08f; setTextIsSelectable(true) },LinearLayout.LayoutParams(0,-2,1f))
                val copy = iconButton(R.drawable.ic_copy,"复制授权码") { }
                copy.setOnClickListener {
                    (getSystemService(CLIPBOARD_SERVICE) as android.content.ClipboardManager).setPrimaryClip(android.content.ClipData.newPlainText("Microsoft 授权码",session.userCode)); toast("授权码已复制")
                }
                codeRow.addView(copy,LinearLayout.LayoutParams(dp(48),dp(48))); codeCard.addView(codeRow)
                codeCard.addView(material.text("复制后在登录页面输入此授权码。",13f).apply { setPadding(0,dp(12),0,0) })
                if (content.childCount > 1) content.removeViewAt(1)
                footer(action("打开 Microsoft 登录页",R.drawable.ic_open,MailUi.Action.Filled,allowWhileBusy = true) {
                    try { startActivity(Intent(Intent.ACTION_VIEW,Uri.parse(session.verificationURI))) }
                    catch (_: android.content.ActivityNotFoundException) { status("无法打开浏览器，请在浏览器访问 ${session.verificationURI} 并输入授权码") }
                })
                status("等待网页授权；完成后会自动登录")
                scheduleOAuthPoll(account.id,session)
            }
            status("正在获取 Microsoft 授权码…")
        }
    }
    private fun scheduleOAuthPoll(id: String, session: DeviceAuthorization) {
        val task = Runnable {
            if (isDestroyed || route != "oauth-login") return@Runnable
            if (busy) { scheduleOAuthPoll(id,session); return@Runnable }
            io({ MicrosoftOAuth.poll(session) },failure = { error -> status(MailRepository.friendly(error)) }) { tokens ->
                if (tokens == null) scheduleOAuthPoll(id,session)
                else io({ repository.saveOAuthTokens(id,session.settings,tokens) }) {
                    clearMailboxFailure(id)
                    unified = false; accountId = id; folder = "INBOX"; folders = emptyList(); messages = emptyList()
                    toast("授权成功"); showInbox(); refreshFolders()
                }
            }
        }
        oauthPollRunnable = task; handler.postDelayed(task,session.intervalSeconds * 1000)
    }
    private fun showConfigImport(localFile: Boolean = false, file: Uri? = null) {
        configLocalFile = localFile; configFileUri = file
        page("导入加密配置","config-import")
        val form = material.form(content)
        val origin = material.card(form,if (localFile) "配置文件" else "配置链接")
        val source = if (localFile) null else input(origin,"HTTPS URL",repository.configURL(),InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI)
        if (localFile) {
            val filename = file?.let { uri -> runCatching { contentResolver.query(uri,arrayOf(OpenableColumns.DISPLAY_NAME),null,null,null)?.use { cursor -> if (cursor.moveToFirst()) cursor.getString(0) else null } }.getOrNull() } ?: "选择密文文件"
            material.listRow(origin,filename,if (file == null) "支持 .txt、.enc、.json 文件" else "点击可重新选择文件",material.emblem(R.drawable.ic_file)) {
                if (busy) return@listRow
                try {
                    startActivityForResult(Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                        addCategory(Intent.CATEGORY_OPENABLE); type = "*/*"
                        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                    },CONFIG_IMPORT)
                } catch (_: android.content.ActivityNotFoundException) { status("无法打开系统文件选择器") }
            }
        }
        val decrypt = material.card(form,"解密配置")
        var cipher = repository.configCipher()
        material.dropdown(decrypt,"加密方式",ConfigCipher.entries.map { it.title },ConfigCipher.entries.indexOf(cipher)) { cipher = ConfigCipher.entries[it] }
        val password = input(decrypt,"解密密码","",InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD).apply { isSaveEnabled = false }
        decrypt.addView(material.text("选择生成文件时使用的加密方式。解密在本机完成，密码不会保存。",13f))
        footer(action(if (localFile) "读取文件并解密" else "下载并解密",R.drawable.ic_lock,MailUi.Action.Filled) {
            val address = source?.text?.toString()?.trim(); val secret = password.text.toString()
            val selectedCipher = cipher
            if (localFile && file == null) { status("请先选择密文文件"); return@action }
            if (secret.isEmpty()) { status("请输入解密密码"); return@action }
            if (address != null) try { RemoteConfig.url(address) } catch (e: MailException) { status(e.message ?: "请输入 HTTPS URL"); return@action }
            password.setText("")
            io({
                if (localFile) contentResolver.openInputStream(file!!)?.use { ConfigFile.read(it,secret,selectedCipher) }
                    ?: throw MailException("无法读取密文文件，请重新选择")
                else RemoteConfig.fetch(address!!,secret,selectedCipher)
            }) { found -> showConfigPreview(address,found,selectedCipher,file) }
            status(if (localFile) "正在读取文件并解密…" else "正在下载并解密…")
        })
    }
    private fun showConfigPreview(address: String?, preview: ConfigPreview, method: ConfigCipher, file: Uri? = null) {
        page("选择要导入的账号","config-preview")
        val form = material.form(content)
        val selected = preview.accounts.map { it.id }.toMutableSet()
        val save = action("导入 ${selected.size} 个邮箱",R.drawable.ic_add,MailUi.Action.Filled) {
            val values = preview.accounts.filter { it.id in selected }
            if (values.isEmpty()) { status("请至少选择一个账号"); return@action }
            io({ repository.importAccounts(values); runCatching { if (address != null) repository.rememberConfigURL(address); repository.rememberConfigCipher(method) } }) {
                if (accountId.isBlank()) accountId = repository.accounts.firstOrNull()?.id ?: ""
                toast("已导入 ${values.size} 个账号"); showAccounts()
            }
        }.apply { isEnabled = selected.isNotEmpty() }
        toolbarAction("重新解密",R.drawable.ic_refresh) { showConfigImport(address == null,file) }
        if (preview.accounts.isEmpty()) material.empty(form,R.drawable.ic_file,"没有可导入的邮箱","请根据下方提示调整配置后重新加密。")
        preview.accounts.forEach { account ->
            val updating = repository.accounts.any { it.importIdentity == account.importIdentity }
            val card = material.card(form)
            val check = MaterialCheckBox(this).apply {
                minHeight = dp(48); isChecked = true; contentDescription = "导入 ${account.title}"
                setOnCheckedChangeListener { _,checked -> if (checked) selected.add(account.id) else selected.remove(account.id); save.text = "导入 ${selected.size} 个邮箱"; save.isEnabled = selected.isNotEmpty() }
            }
            material.listRow(card,account.title,account.email,providerLogo(account,40),check) { check.isChecked = !check.isChecked }
            card.addView(material.text(if (updating) "更新已有邮箱" else "添加新邮箱",12f,MailTheme.primary))
            card.addView(material.text("IMAP ${account.imapHost}:${account.imapPort}\nSMTP ${account.smtpHost}:${account.smtpPort}",12f).apply { setPadding(0,dp(8),0,0) })
        }
        if (preview.warnings.isNotEmpty()) material.card(form,"导入提示").addView(material.text(preview.warnings.joinToString("\n\n"),13f,MailTheme.warning))
        form.addView(material.text("收发邮件时验证连接；待授权邮箱保存后再登录。",13f).apply { setPadding(dp(4),dp(4),dp(4),dp(8)) })
        footer(save)
    }
    private fun showProviderPicker() {
        page("添加邮箱","provider-picker")
        val form = material.form(content)
        form.addView(material.text("选择邮箱服务商",24f,MailTheme.text,true).apply { setPadding(dp(4),dp(4),dp(4),dp(8)) })
        form.addView(material.text("服务器设置会自动填好。",14f).apply { setPadding(dp(4),0,dp(4),dp(20)) })
        listOf("QQ","163","126","Gmail","Outlook","阿里云").forEach { name ->
            val account = MailAccount().apply { preset(name) }
            val card = material.card(form)
            material.listRow(card,name,if (name == "Outlook") "授权登录" else "邮箱地址与授权码",providerLogo(account,40),iconButton(R.drawable.ic_next,"选择 $name") { showAccountEditor(account,providerName = name) }) { showAccountEditor(account,providerName = name) }
        }
        val other = material.card(form)
        material.listRow(other,"其他邮箱","填写自定义服务器",material.emblem(R.drawable.ic_mail)) { showAccountEditor(MailAccount(),providerName = "其他邮箱",forceAdvanced = true) }
    }
    private fun showAccountEditor(account: MailAccount, providerName: String? = null, forceAdvanced: Boolean = false) {
        val existing = repository.accounts.any { it.id == account.id }
        page(if (existing) "邮箱设置" else providerName ?: "添加邮箱",if (existing) "account-edit" else "account-new")
        val form = material.form(content)
        if (providerName != null) material.listRow(material.card(form),providerName,"",providerLogo(account,40))
        val identity = material.card(form,"邮箱信息")
        val email = input(identity,"邮箱地址",account.email,InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS)
        val display = input(identity,"显示名称（可选）",account.displayName)
        val login = material.card(form,"登录方式")
        val secret = input(login,if (account.oauth2) "已有访问令牌（可选）" else "密码 / 客户端授权码",account.credential,InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD)
        if (account.oauth2) login.addView(material.text("无需填写密码或令牌。保存后点击邮箱进行授权登录。",13f))
        val advancedToggle = material.action("高级设置",R.drawable.ic_expand)
        form.addView(advancedToggle,LinearLayout.LayoutParams(-1,-2).apply { bottomMargin = dp(12) })
        val advanced = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; visibility = if (forceAdvanced || account.imapHost.isBlank()) View.VISIBLE else View.GONE; form.addView(this) }
        fun updateAdvancedLabel() { advancedToggle.text = if (advanced.visibility == View.VISIBLE) "收起高级设置" else "高级设置"; advancedToggle.setIconResource(if (advanced.visibility == View.VISIBLE) R.drawable.ic_collapse else R.drawable.ic_expand) }
        updateAdvancedLabel()
        advancedToggle.setOnClickListener { if (!busy) { advanced.visibility = if (advanced.visibility == View.VISIBLE) View.GONE else View.VISIBLE; updateAdvancedLabel() } }
        val advancedLogin = material.card(advanced,"高级登录设置")
        val user = input(advancedLogin,"登录用户名（留空使用邮箱）",account.username,InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS)
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
        login.addView(action("从密码管理器填入",R.drawable.ic_lock) {
            val manager = getSystemService(AutofillManager::class.java)
            if (manager == null || !manager.isEnabled) {
                status("在系统设置中启用“密码自动填充”并选择密码管理器，再返回此处重试")
            } else {
                secret.requestFocus()
                manager.requestAutofill(secret)
            }
        })
        login.addView(material.text("QQ、163 等邮箱请先开启 IMAP/SMTP，并使用客户端授权码。",13f))
        val oauth = material.switch(advancedLogin,"OAuth 授权登录","Outlook 可先保存，再授权登录",account.oauth2)
        val oauthForm = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; visibility = if (oauth.isChecked) View.VISIBLE else View.GONE; advancedLogin.addView(this) }
        val clientId = input(oauthForm,"OAuth Client ID（应用 ID）",account.oauthSettings?.clientId ?: "")
        val tokenEndpoint = input(oauthForm,"Microsoft OAuth 令牌端点",account.oauthSettings?.tokenEndpoint ?: MicrosoftOAuth.DEFAULT_ENDPOINT,InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI)
        oauthForm.addView(material.text("Client ID 来自邮箱配置的 OAuth 应用。Outlook 可留空访问令牌，保存后点击邮箱登录。",13f))
        oauth.setOnCheckedChangeListener { _,checked -> oauthForm.visibility = if (checked) View.VISIBLE else View.GONE }
        val presetCard = material.card(advanced,"服务器配置")
        val imapCard = material.card(advanced,"收信 · IMAP")
        val imap = input(imapCard,"服务器地址",account.imapHost,InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI)
        val imapPort = input(imapCard,"端口",account.imapPort.toString(),InputType.TYPE_CLASS_NUMBER)
        val imapUser = input(imapCard,"专用用户名（可留空）",account.imapUsername ?: "")
        val imapStart = material.switch(imapCard,"STARTTLS","关闭时使用隐式 TLS，默认端口 993",account.imapStartTLS)
        val smtpCard = material.card(advanced,"发信 · SMTP")
        val smtp = input(smtpCard,"服务器地址",account.smtpHost,InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI)
        val smtpPort = input(smtpCard,"端口",account.smtpPort.toString(),InputType.TYPE_CLASS_NUMBER)
        val smtpUser = input(smtpCard,"专用用户名（可留空）",account.smtpUsername ?: "")
        val smtpStart = material.switch(smtpCard,"STARTTLS","关闭时使用隐式 TLS，默认端口 465",account.smtpStartTLS)
        imapStart.setOnCheckedChangeListener { _,checked -> imapPort.setText(if (checked) "143" else "993") }; smtpStart.setOnCheckedChangeListener { _,checked -> smtpPort.setText(if (checked) "587" else "465") }
        val names = listOf("自定义配置","QQ","163","126","Gmail","阿里云","Outlook")
        material.dropdown(presetCard,"服务商预设",names,0) { i ->
            if (i > 0) {
                account.preset(names[i]); imap.setText(account.imapHost); smtp.setText(account.smtpHost); imapStart.isChecked = account.imapStartTLS; smtpStart.isChecked = account.smtpStartTLS
                imapPort.setText(String.format(java.util.Locale.ROOT,"%d",account.imapPort)); smtpPort.setText(String.format(java.util.Locale.ROOT,"%d",account.smtpPort)); oauth.isChecked = account.oauth2
            }
        }
        presetCard.addView(material.text("选择预设可填入服务器和端口，也可手动修改。",13f))
        footer(action("保存邮箱",R.drawable.ic_mail,MailUi.Action.Filled) {
            val oldSettings = account.oauthSettings; val oldCredential = account.credential
            account.email = email.text.toString().trim(); account.displayName = display.text.toString(); account.username = user.text.toString().trim(); account.credential = secret.text.toString(); account.oauth2 = oauth.isChecked
            account.imapHost = imap.text.toString().trim(); account.smtpHost = smtp.text.toString().trim(); account.imapPort = imapPort.text.toString().toIntOrNull() ?: 0; account.smtpPort = smtpPort.text.toString().toIntOrNull() ?: 0
            account.imapUsername = imapUser.text.toString().takeIf { it.isNotEmpty() }; account.smtpUsername = smtpUser.text.toString().takeIf { it.isNotEmpty() }
            account.imapStartTLS = imapStart.isChecked; account.smtpStartTLS = smtpStart.isChecked
            if (account.oauth2 && (clientId.text.isNotBlank() || account.credential.isEmpty() || oldSettings != null)) {
                val endpoint = tokenEndpoint.text.toString().trim()
                try { MicrosoftOAuth.endpoint(endpoint) } catch (e: MailException) { status(e.message ?: "OAuth 配置无效"); return@action }
                val settings = OAuthSettings(clientId.text.toString().trim(),endpoint,oldSettings?.scopes ?: MicrosoftOAuth.DEFAULT_SCOPES)
                val same = oldSettings != null && oldSettings.clientId == settings.clientId && oldSettings.tokenEndpoint == settings.tokenEndpoint
                account.oauthSettings = if (same && account.credential == oldCredential) oldSettings else settings
                if (!same && oldSettings != null) account.credential = ""
            } else account.oauthSettings = null
            val value = account.copy(); disableTree(form)
            io({ repository.saveAccount(value) },failure = { error -> showAccountEditor(value,providerName,advanced.visibility == View.VISIBLE); status(MailRepository.friendly(error)) },cancellable = true) {
                getSystemService(AutofillManager::class.java)?.commit(); unified = false; accountId = value.id; folder = "INBOX"; folders = repository.cachedFolders(accountId); messages = emptyList()
                if (value.needsAuthorization) showOAuthLogin(value) else { showInbox(); reload() }
            }
        })
    }
    private fun showComposer(initial: Draft) {
        page(if (initial.inReplyTo.isEmpty()) "写邮件" else "回复邮件","compose")
        composeDraft = initial
        val form = material.form(content)
        val selected = repository.accounts.firstOrNull { it.id == initial.accountId }
        val from = material.card(form,"发件邮箱")
        if (initial.inReplyTo.isNotEmpty()) {
            if (selected != null) material.listRow(from,selected.title,"回复使用此邮箱发送",providerLogo(selected,40))
        } else {
            val accounts = repository.accounts.toList()
            material.dropdown(from,"邮箱",accounts.map { if (it.title == it.email) it.email else "${it.title} · ${it.email}" },accounts.indexOfFirst { it.id == initial.accountId }.coerceAtLeast(0)) { index ->
                initial.accountId = accounts[index].id; persistDraft()
            }
        }
        val recipients = material.card(form,"收件人")
        val to = input(recipients,"收件地址",initial.to,InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS)
        val copies = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; visibility = if (initial.cc.isNotEmpty() || initial.bcc.isNotEmpty()) View.VISIBLE else View.GONE }
        val copiesAction = action("抄送 / 密送",R.drawable.ic_add) { copies.visibility = if (copies.visibility == View.VISIBLE) View.GONE else View.VISIBLE }
        recipients.addView(copiesAction); recipients.addView(copies)
        val cc = input(copies,"抄送",initial.cc,InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS)
        val bcc = input(copies,"密送",initial.bcc,InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS)
        val message = material.card(form)
        val subject = input(message,"主题",initial.subject)
        val body = TextInputEditText(this).apply {
            importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO; inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_MULTI_LINE or InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
            setText(initial.text); hint = "开始写邮件…"; contentDescription = "正文"; background = null; textSize = 16f; minLines = 10; gravity = Gravity.TOP
            setTextColor(MailTheme.text); setHintTextColor(MailTheme.muted); setLineSpacing(dp(4).toFloat(),1f); setPadding(dp(4),dp(12),dp(4),dp(8))
        }
        message.addView(body,LinearLayout.LayoutParams(-1,-2))
        val watcher = object : TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) { }
            override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {
                initial.to = to.text.toString(); initial.cc = cc.text.toString(); initial.bcc = bcc.text.toString(); initial.subject = subject.text.toString(); initial.text = body.text.toString()
                handler.removeCallbacks(saveRunnable); handler.postDelayed(saveRunnable,350)
            }
            override fun afterTextChanged(s: Editable?) { }
        }
        listOf(to,cc,bcc,subject,body).forEach { it.addTextChangedListener(watcher) }
        val attachments = material.card(form,"附件")
        val chips = ChipGroup(this); attachments.addView(chips)
        fun renderAttachments() {
            (attachments.parent as View).visibility = if (initial.attachments.isEmpty()) View.GONE else View.VISIBLE
            chips.removeAllViews()
            initial.attachments.forEach { attachment -> chips.addView(Chip(this).apply {
                text = attachment.filename; maxWidth = resources.displayMetrics.widthPixels - dp(96); ellipsize = TextUtils.TruncateAt.END
                chipIcon = AppCompatResources.getDrawable(this@MainActivity,R.drawable.ic_attachment); isChipIconVisible = true; isCloseIconVisible = true; closeIcon = AppCompatResources.getDrawable(this@MainActivity,R.drawable.ic_close)
                closeIconContentDescription = "移除附件 ${attachment.filename}"
                setOnCloseIconClickListener { if (!busy) { initial.attachments = initial.attachments.filter { it.id != attachment.id }; persistDraft(); renderAttachments() } }
            }) }
        }
        renderAttachments()
        attachments.addView(material.text("单个附件最多 15 MB，合计最多 18 MB。",12f))
        form.addView(material.text("草稿自动保存到本机",12f).apply { gravity = Gravity.CENTER; setPadding(0,dp(4),0,dp(8)) })
        toolbarAction("添加附件",R.drawable.ic_attachment) { persistDraft(); startActivityForResult(Intent(Intent.ACTION_OPEN_DOCUMENT).apply { addCategory(Intent.CATEGORY_OPENABLE); type = "*/*"; putExtra(Intent.EXTRA_ALLOW_MULTIPLE,true) },IMPORT) }
        toolbarAction("发送",R.drawable.ic_send) { sendDraft(initial,form) }
        moreAction { popup ->
            popupItem(popup,"保存并关闭",R.drawable.ic_compose) { persistDraft(); composeDraft = null; showDrafts() }
            popupItem(popup,"丢弃草稿",R.drawable.ic_delete,true) { discardDraft(initial) }
        }
    }
    private fun discardDraft(draft: Draft) {
        MaterialAlertDialogBuilder(this).setTitle("丢弃草稿？").setMessage("此草稿及其附件将从本机移除。")
            .setPositiveButton("丢弃") { _,_ ->
                try { repository.removeDraft(draft.id); if (composeDraft?.id == draft.id) { handler.removeCallbacks(saveRunnable); composeDraft = null }; showDrafts() }
                catch (error: Exception) { status(MailRepository.friendly(error)) }
            }.setNegativeButton("取消",null).show()
    }
    private fun sendDraft(initial: Draft, form: View) {
        try { initial.validate(); persistDraft() } catch (error: Exception) { status(MailRepository.friendly(error)); return }
        // Detach the editor before delivery so lifecycle/autosave cannot overwrite its durable status.
        handler.removeCallbacks(saveRunnable); composeDraft = null; disableTree(form)
        for (index in 0 until toolbar.menu.size()) toolbar.menu.getItem(index).isEnabled = false
        val outgoing = initial.copy()
        io({ repository.send(outgoing) },failure = { error ->
            toast(MailRepository.friendly(error))
            if (repository.drafts.any { it.id == initial.id && it.delivery != "draft" }) showOutbox() else showComposer(initial)
            status(MailRepository.friendly(error))
        }) { outcome -> toast(outcome); showOutbox() }
        status("正在发送；完成后可在发件记录查看结果")
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
        if (request == CONFIG_IMPORT) {
            data.data?.let { showConfigImport(localFile = true,file = it) }
            return
        }
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
    companion object { @Volatile private var sharedRepository: MailRepository? = null; private const val IMPORT = 40; private const val EXPORT = 41; private const val CONFIG_IMPORT = 42 }
}
