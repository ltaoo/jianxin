package com.jianxin.mail

import android.content.Context
import android.content.res.ColorStateList
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.text.InputType
import android.text.TextUtils
import android.view.Gravity
import android.view.View
import android.widget.*
import com.google.android.material.button.MaterialButton
import com.google.android.material.card.MaterialCardView
import com.google.android.material.materialswitch.MaterialSwitch
import com.google.android.material.textfield.MaterialAutoCompleteTextView
import com.google.android.material.textfield.TextInputEditText
import com.google.android.material.textfield.TextInputLayout

// Shared Material 3 layout primitives for the native management and editing screens.
class MailUi(private val context: Context) {
    enum class Action { Filled, Tonal, Text }
    private fun dp(value: Int) = (value * context.resources.displayMetrics.density).toInt()
    fun text(value: String, size: Float = 14f, color: Int = MailTheme.muted, bold: Boolean = false) = TextView(context).apply {
        text = value; textSize = size; setTextColor(color); setLineSpacing(dp(3).toFloat(),1f)
        if (bold) setTypeface(typeface,Typeface.BOLD)
    }
    fun form(content: LinearLayout): LinearLayout {
        val form = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(16),dp(12),dp(16),dp(24)) }
        val scroll = ScrollView(context).apply { isFillViewport = true; clipToPadding = false; addView(form) }
        content.addView(scroll,LinearLayout.LayoutParams(-1,0,1f)); return form
    }
    fun card(parent: LinearLayout, title: String = ""): LinearLayout {
        val card = MaterialCardView(context).apply { radius = dp(24).toFloat(); cardElevation = 0f; strokeWidth = 0; setCardBackgroundColor(MailTheme.recessed) }
        val body = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(16),dp(16),dp(16),dp(16)) }
        if (title.isNotEmpty()) body.addView(text(title,14f,MailTheme.primary,true),LinearLayout.LayoutParams(-1,-2).apply { bottomMargin = dp(8) })
        card.addView(body); parent.addView(card,LinearLayout.LayoutParams(-1,-2).apply { bottomMargin = dp(16) }); return body
    }
    fun action(label: String, icon: Int = 0, appearance: Action = Action.Text) = MaterialButton(context).apply {
        text = label; isAllCaps = false; minHeight = dp(48); cornerRadius = dp(24); strokeWidth = 0; insetTop = dp(4); insetBottom = dp(4)
        backgroundTintList = ColorStateList.valueOf(when (appearance) { Action.Filled -> MailTheme.primary; Action.Tonal -> MailTheme.selected; Action.Text -> android.graphics.Color.TRANSPARENT })
        setTextColor(if (appearance == Action.Filled) MailTheme.primaryForeground else MailTheme.primary)
        if (icon != 0) setIconResource(icon)
        iconGravity = MaterialButton.ICON_GRAVITY_TEXT_START
        iconSize = dp(20); iconTint = ColorStateList.valueOf(if (appearance == Action.Filled) MailTheme.primaryForeground else MailTheme.primary)
    }
    fun field(parent: LinearLayout, label: String, value: String, type: Int): TextInputEditText {
        val layout = TextInputLayout(context,null,com.google.android.material.R.attr.textInputOutlinedStyle).apply {
            hint = label; boxBackgroundMode = TextInputLayout.BOX_BACKGROUND_OUTLINE
            setBoxCornerRadii(dp(16).toFloat(),dp(16).toFloat(),dp(16).toFloat(),dp(16).toFloat())
            if (type and InputType.TYPE_MASK_VARIATION == InputType.TYPE_TEXT_VARIATION_PASSWORD) endIconMode = TextInputLayout.END_ICON_PASSWORD_TOGGLE
        }
        parent.addView(layout,LinearLayout.LayoutParams(-1,-2).apply { topMargin = dp(8); bottomMargin = dp(8) })
        return TextInputEditText(layout.context).apply {
            importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO; inputType = type; setText(value); minHeight = dp(56); contentDescription = label; layout.addView(this)
            textSize = 16f; setPadding(dp(16),dp(14),dp(16),dp(14))
        }
    }
    fun dropdown(parent: LinearLayout, label: String, items: List<String>, selected: Int, onSelect: (Int) -> Unit): MaterialAutoCompleteTextView {
        val layout = TextInputLayout(context,null,com.google.android.material.R.attr.textInputOutlinedStyle).apply {
            hint = label; boxBackgroundMode = TextInputLayout.BOX_BACKGROUND_OUTLINE; endIconMode = TextInputLayout.END_ICON_DROPDOWN_MENU
            setBoxCornerRadii(dp(16).toFloat(),dp(16).toFloat(),dp(16).toFloat(),dp(16).toFloat())
        }
        val field = MaterialAutoCompleteTextView(layout.context).apply {
            inputType = InputType.TYPE_NULL; keyListener = null; minHeight = dp(56); contentDescription = label; importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO
            textSize = 16f; setPadding(dp(16),dp(14),dp(16),dp(14))
            setAdapter(ArrayAdapter(context,R.layout.mail_dropdown_item,items)); setText(items.getOrElse(selected) { "" },false)
            setOnItemClickListener { _,_,position,_ -> onSelect(position) }
        }
        layout.addView(field); parent.addView(layout,LinearLayout.LayoutParams(-1,-2).apply { topMargin = dp(8); bottomMargin = dp(8) }); return field
    }
    fun switch(parent: LinearLayout, title: String, subtitle: String, checked: Boolean): MaterialSwitch {
        val row = LinearLayout(context).apply { gravity = Gravity.CENTER_VERTICAL; minimumHeight = dp(64); setPadding(0,dp(8),0,dp(8)) }
        val labels = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
        labels.addView(text(title,16f,MailTheme.text)); if (subtitle.isNotEmpty()) labels.addView(text(subtitle,12f).apply { setPadding(0,dp(4),dp(8),0) })
        row.addView(labels,LinearLayout.LayoutParams(0,-2,1f))
        val toggle = MaterialSwitch(context).apply { isChecked = checked; contentDescription = title; minHeight = dp(48) }
        row.addView(toggle); row.setOnClickListener { toggle.isChecked = !toggle.isChecked }; parent.addView(row); return toggle
    }
    fun listRow(parent: LinearLayout, title: String, subtitle: String, leading: View, trailing: View? = null, action: (() -> Unit)? = null): LinearLayout {
        val row = LinearLayout(context).apply { gravity = Gravity.CENTER_VERTICAL; minimumHeight = dp(72); setPadding(0,dp(8),0,dp(8)) }
        row.addView(leading,LinearLayout.LayoutParams(dp(40),dp(40)).apply { marginEnd = dp(16) })
        val labels = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
        labels.addView(text(title,16f,MailTheme.text,true).apply { maxLines = 2; ellipsize = TextUtils.TruncateAt.END })
        if (subtitle.isNotEmpty()) labels.addView(text(subtitle,13f).apply { maxLines = 3; ellipsize = TextUtils.TruncateAt.END; setPadding(0,dp(4),0,0) })
        row.addView(labels,LinearLayout.LayoutParams(0,-2,1f))
        trailing?.let { row.addView(it,LinearLayout.LayoutParams(dp(48),dp(48))) }
        action?.let { callback -> row.isFocusable = true; row.setOnClickListener { callback() }; val appearance = context.obtainStyledAttributes(intArrayOf(android.R.attr.selectableItemBackground)); row.background = appearance.getDrawable(0); appearance.recycle() }
        parent.addView(row); return row
    }
    fun emblem(icon: Int) = ImageView(context).apply {
        setImageResource(icon); imageTintList = ColorStateList.valueOf(MailTheme.primary); setPadding(dp(10),dp(10),dp(10),dp(10))
        background = GradientDrawable().apply { shape = GradientDrawable.OVAL; setColor(MailTheme.selected) }; importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
    }
    fun empty(parent: LinearLayout, icon: Int, title: String, message: String) {
        val body = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL; gravity = Gravity.CENTER; setPadding(dp(24),dp(48),dp(24),dp(32)) }
        body.addView(emblem(icon),LinearLayout.LayoutParams(dp(64),dp(64)))
        body.addView(text(title,22f,MailTheme.text,true).apply { gravity = Gravity.CENTER },LinearLayout.LayoutParams(-1,-2).apply { topMargin = dp(20); bottomMargin = dp(8) })
        body.addView(text(message).apply { gravity = Gravity.CENTER }); parent.addView(body)
    }
}
