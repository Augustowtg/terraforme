package com.terraforme.remote.ui.screens

import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.terraforme.remote.data.ChatMessage

// As cores do chat do terra no PC (Catppuccin Mocha, src/theme.rs).
private val Text = Color(0xFFCDD6F4)
private val Subtext0 = Color(0xFFA6ADC8)
private val Overlay0 = Color(0xFF6C7086)
private val Mauve = Color(0xFFCBA6F7)
private val Green = Color(0xFFA6E3A1)
private val Red = Color(0xFFF38BA8)
private val UserBorder = Color(0xFF9E2A4F)

private val Mono = FontFamily.Monospace
private val Size = 13.sp

/** Pensamento, ferramenta e resultado: os passos, que formam a árvore do PC. */
private fun isStep(m: ChatMessage) = m.role in setOf("info", "tool", "tool_result", "tool_error")
private fun isResult(m: ChatMessage) = m.role == "tool_result" || m.role == "tool_error"

/**
 * O galho de cada mensagem, como no `history.rs` do terra: o primeiro passo
 * é a cabeça, os seguintes `├─`/`└─`, e o resultado fica embaixo da ferramenta.
 */
internal fun branches(history: List<ChatMessage>): List<String> = history.indices.map { i ->
    if (!isStep(history[i]) || i == 0 || !isStep(history[i - 1])) return@map ""
    val next = history.drop(i + 1).firstOrNull { !isResult(it) }
    val last = next == null || !isStep(next)
    when {
        isResult(history[i]) -> if (last) "   " else "│  "
        last -> "└─ "
        else -> "├─ "
    }
}

/** O chat de uma aba, desenhado como no terra do PC. */
@Composable
fun ChatView(history: List<ChatMessage>, modifier: Modifier = Modifier) {
    // O "Preparando Bash..." vira a própria ferramenta no PC; gravado, só sobra o último.
    val shown = history.filterIndexed { i, m ->
        !(m.role == "info" && m.content.startsWith("Preparando ") && i != history.lastIndex)
    }
    val branch = branches(shown)
    val list = rememberLazyListState()
    LaunchedEffect(shown.size) { if (shown.isNotEmpty()) list.scrollToItem(shown.lastIndex) }
    LazyColumn(state = list, modifier = modifier) {
        itemsIndexed(shown) { i, m ->
            // Linha em branco entre as entradas, como no PC; resultado e galhos ficam colados.
            val gap = if (i > 0 && branch[i].isEmpty() && !isResult(m)) 10.dp else 0.dp
            Row(Modifier.padding(top = gap)) {
                if (branch[i].isNotEmpty()) Text(branch[i], color = Overlay0, fontFamily = Mono, fontSize = Size)
                Entry(m)
            }
        }
    }
}

@Composable
private fun Entry(m: ChatMessage) {
    when (m.role) {
        "user" -> Text(
            m.content, color = Color.White, fontWeight = FontWeight.Bold, fontFamily = Mono, fontSize = Size,
            modifier = Modifier.fillMaxWidth().border(1.dp, UserBorder, RoundedCornerShape(6.dp)).padding(10.dp)
        )
        "assistant" -> Text(
            m.content.trim(), color = Text, fontFamily = Mono, fontSize = Size,
            modifier = Modifier.fillMaxWidth().border(1.dp, Overlay0, RoundedCornerShape(6.dp)).padding(10.dp)
        )
        "tool" -> Text(
            buildAnnotatedString {
                withStyle(SpanStyle(color = Green)) { append("${toolIcon(m.tool.orEmpty())} ") }
                withStyle(SpanStyle(color = Text, fontWeight = FontWeight.Bold)) { append(m.tool.orEmpty()) }
                if (m.content.isNotEmpty()) withStyle(SpanStyle(color = Subtext0)) { append("(${m.content})") }
            },
            fontFamily = Mono, fontSize = Size
        )
        "tool_result", "tool_error" -> Row(horizontalArrangement = Arrangement.Start) {
            Text("  ⎿ ", color = Overlay0, fontFamily = Mono, fontSize = Size)
            Text(
                m.content.ifEmpty { "(sem saída)" },
                color = if (m.role == "tool_error") Red else Overlay0,
                fontFamily = Mono, fontSize = Size, maxLines = 6
            )
        }
        "error" -> Prefixed("✘ ", m.content, Red, Red)
        else -> when {
            m.content.startsWith("Pensamento: ") ->
                Prefixed("💡 ", m.content.removePrefix("Pensamento: "), Mauve, Overlay0, italic = true)
            m.content.startsWith("Subagente: ") ->
                Prefixed("🤖 ", m.content.removePrefix("Subagente: "), Mauve, Overlay0, italic = true)
            m.content.startsWith("Preparando ") -> Prefixed("⏳ ", m.content, Overlay0, Overlay0)
            m.content == "Pensando..." -> Prefixed("📌 ", m.content, Overlay0, Overlay0)
            else -> Prefixed("· ", m.content, Overlay0, Overlay0)
        }
    }
}

@Composable
private fun Prefixed(prefix: String, text: String, prefixColor: Color, color: Color, italic: Boolean = false) {
    Text(
        buildAnnotatedString {
            withStyle(SpanStyle(color = prefixColor)) { append(prefix) }
            withStyle(SpanStyle(color = color, fontStyle = if (italic) FontStyle.Italic else null)) { append(text) }
        },
        fontFamily = Mono, fontSize = Size
    )
}

/** O ícone da ferramenta, o mesmo do `tool_icon` do terra. */
internal fun toolIcon(name: String): String {
    val n = name.lowercase()
    fun has(vararg w: String) = w.any { it in n }
    return when {
        n.startsWith("mcp__") -> "🔌"
        has("bash", "shell", "command", "terminal") -> "💻"
        has("agent", "task") -> "🤖"
        has("web", "fetch", "url") -> "🌐"
        has("edit", "write", "patch", "replace") -> "📝"
        has("grep", "glob", "search", "find", "list") -> "🔍"
        has("read", "view") -> "📄"
        has("todo") -> "📋"
        else -> "🔧"
    }
}
