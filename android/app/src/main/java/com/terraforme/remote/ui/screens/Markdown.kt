package com.terraforme.remote.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

// O markdown das respostas como no terra do PC (src/tui/history/markdown.rs e
// code.rs): títulos, listas, citações, **negrito**, *itálico* e `código`, e os
// blocos ``` numa caixa com a linguagem em cima e o código realçado.

// Catppuccin Mocha, como o src/theme.rs.
private val MdText = Color(0xFFCDD6F4)
private val Subtext1 = Color(0xFFBAC2DE)
private val Overlay0 = Color(0xFF6C7086)
private val Surface1 = Color(0xFF45475A)
private val Base = Color(0xFF1E1E2E)
private val Mauve = Color(0xFFCBA6F7)
private val Green = Color(0xFFA6E3A1)
private val Yellow = Color(0xFFF9E2AF)
private val Lavender = Color(0xFFB4BEFE)
private val Subtext0 = Color(0xFFA6ADC8)

private val Mono = FontFamily.Monospace
private val Size = 13.sp

/** Um trecho da resposta: texto corrido ou bloco cercado por ```. */
internal sealed interface Segment {
    data class Prose(val lines: List<String>) : Segment
    data class Fence(val lang: String, val lines: List<String>) : Segment
}

/** Separa os blocos ```; um bloco ainda aberto (streaming) vai até o fim. */
internal fun segments(text: String): List<Segment> {
    val out = mutableListOf<Segment>()
    val prose = mutableListOf<String>()
    var lang: String? = null
    val code = mutableListOf<String>()
    for (line in text.split('\n')) {
        val fenced = line.trimStart().takeIf { it.startsWith("```") }?.removePrefix("```")
        when {
            lang != null && fenced != null && fenced.isBlank() -> {
                out += Segment.Fence(lang, code.toList()); code.clear(); lang = null
            }
            lang != null -> code += line
            fenced != null -> {
                if (prose.isNotEmpty()) { out += Segment.Prose(prose.toList()); prose.clear() }
                lang = fenced.trim()
            }
            else -> prose += line
        }
    }
    if (lang != null) out += Segment.Fence(lang, code.toList())
    if (prose.isNotEmpty()) out += Segment.Prose(prose.toList())
    return out
}

@Composable
internal fun Markdown(text: String, modifier: Modifier = Modifier) {
    Column(modifier) {
        segments(text).forEach { segment ->
            when (segment) {
                is Segment.Prose -> Text(
                    buildAnnotatedString {
                        segment.lines.forEachIndexed { i, line ->
                            if (i > 0) append('\n')
                            append(proseLine(line))
                        }
                    },
                    color = MdText, fontFamily = Mono, fontSize = Size
                )
                is Segment.Fence -> CodeBox(segment.lang, segment.lines)
            }
        }
    }
}

/** A caixa de borda arredondada, como a do input, com a linguagem na borda de cima. */
@Composable
private fun CodeBox(lang: String, lines: List<String>) {
    val markdown = lang == "md" || lang == "markdown"
    val body = if (markdown) {
        buildAnnotatedString { lines.forEachIndexed { i, l -> if (i > 0) append('\n'); append(proseLine(l)) } }
    } else {
        highlight(lang, lines)
    }
    Spacer(Modifier.height(4.dp))
    Box(Modifier.fillMaxWidth().padding(top = 8.dp, bottom = 4.dp)) {
        Text(
            body, color = MdText, fontFamily = Mono, fontSize = Size,
            modifier = Modifier.fillMaxWidth()
                .border(1.dp, Overlay0, RoundedCornerShape(6.dp))
                .padding(start = 10.dp, end = 10.dp, top = 12.dp, bottom = 8.dp)
        )
        Text(
            lang.ifEmpty { "código" }, color = Mauve, fontWeight = FontWeight.Bold,
            fontFamily = Mono, fontSize = Size,
            modifier = Modifier.offset(x = 10.dp, y = (-9).dp).background(Base).padding(horizontal = 4.dp)
        )
    }
}

/** Título, citação, item de lista, linha horizontal ou texto, sem os marcadores. */
internal fun proseLine(line: String): AnnotatedString {
    val trimmed = line.trimStart()
    val lead = line.substring(0, line.length - trimmed.length)
    val hashes = trimmed.takeWhile { it == '#' }.length
    if (hashes in 1..6 && trimmed.drop(hashes).startsWith(" ")) {
        return inline(trimmed.drop(hashes).trimStart(), SpanStyle(color = Mauve, fontWeight = FontWeight.Bold))
    }
    if (trimmed.startsWith("> ")) {
        return buildAnnotatedString {
            withStyle(SpanStyle(color = Overlay0)) { append("▎ ") }
            append(inline(trimmed.drop(2), SpanStyle(color = Overlay0, fontStyle = FontStyle.Italic)))
        }
    }
    listOf("- ", "* ", "+ ").firstOrNull { trimmed.startsWith(it) }?.let { marker ->
        return buildAnnotatedString {
            append(lead)
            withStyle(SpanStyle(color = Mauve)) { append("• ") }
            append(inline(trimmed.removePrefix(marker), SpanStyle(color = MdText)))
        }
    }
    if (trimmed in setOf("---", "***", "___")) {
        return buildAnnotatedString { withStyle(SpanStyle(color = Overlay0)) { append("─".repeat(20)) } }
    }
    return inline(line, SpanStyle(color = MdText))
}

/** Negrito (`**`), itálico (`*`/`_`) e `código` dentro de uma linha. */
internal fun inline(text: String, base: SpanStyle): AnnotatedString = buildAnnotatedString {
    var bold = false
    var italic = false
    var code = false
    fun closes(from: Int, marker: String) = text.indexOf(marker, from) >= 0
    var i = 0
    while (i < text.length) {
        val c = text[i]
        if (c == '`' && (code || closes(i + 1, "`"))) { code = !code; i++; continue }
        if (!code) {
            if (text.startsWith("**", i) && (bold || closes(i + 2, "**"))) { bold = !bold; i += 2; continue }
            // `_` só marca itálico na borda da palavra (snake_case fica).
            fun alnum(k: Int) = k in text.indices && text[k].isLetterOrDigit()
            val edge = c == '*' || (c == '_' && if (italic) !alnum(i + 1) else !alnum(i - 1))
            val opens = i + 1 < text.length && !text[i + 1].isWhitespace() && closes(i + 1, c.toString())
            if ((c == '*' || c == '_') && edge && (italic || opens)) { italic = !italic; i++; continue }
        }
        val style = when {
            code -> SpanStyle(color = Subtext1, background = Surface1)
            else -> base.merge(
                SpanStyle(
                    fontWeight = if (bold) FontWeight.Bold else null,
                    fontStyle = if (italic) FontStyle.Italic else null
                )
            )
        }
        withStyle(style) { append(c) }
        i++
    }
}

// Realce dos blocos de código, como o src/tui/history/code.rs.

private class Lang(val line: List<String>, val block: Pair<String, String>?, val keywords: Set<String>)

private val RUST = setOf(
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while",
)
private val PYTHON = setOf(
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "False", "finally", "for", "from", "global", "if", "import", "in", "is",
    "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "self", "True", "try",
    "while", "with", "yield",
)
private val JS = setOf(
    "async", "await", "break", "case", "catch", "class", "const", "continue", "default", "delete",
    "do", "else", "enum", "export", "extends", "false", "finally", "for", "from", "function", "if",
    "implements", "import", "in", "instanceof", "interface", "let", "new", "null", "of", "private",
    "protected", "public", "readonly", "return", "static", "super", "switch", "this", "throw",
    "true", "try", "type", "typeof", "undefined", "var", "void", "while", "yield",
)
private val GO = setOf(
    "break", "case", "chan", "const", "continue", "default", "defer", "else", "false", "for",
    "func", "go", "goto", "if", "import", "interface", "map", "nil", "package", "range", "return",
    "select", "struct", "switch", "true", "type", "var",
)
private val C = setOf(
    "auto", "break", "case", "catch", "class", "const", "continue", "default", "delete", "do",
    "else", "enum", "extern", "false", "final", "for", "if", "import", "namespace", "new", "null",
    "nullptr", "package", "private", "protected", "public", "return", "static", "struct", "switch",
    "this", "throw", "true", "try", "typedef", "union", "using", "virtual", "void", "volatile",
    "while",
)
private val SHELL = setOf(
    "case", "do", "done", "elif", "else", "esac", "export", "fi", "for", "function", "if", "in",
    "local", "return", "then", "until", "while",
)
private val SQL = setOf(
    "and", "as", "by", "create", "delete", "from", "group", "insert", "into", "join", "left",
    "limit", "not", "null", "on", "or", "order", "select", "set", "table", "update", "values",
    "where",
)
private val DATA = setOf("true", "false", "null")

private fun lang(name: String): Lang {
    val slashes = listOf("//")
    val cBlock = "/*" to "*/"
    return when (name.lowercase()) {
        "rust", "rs" -> Lang(slashes, cBlock, RUST)
        "python", "py" -> Lang(listOf("#"), null, PYTHON)
        "js", "javascript", "ts", "typescript", "jsx", "tsx" -> Lang(slashes, cBlock, JS)
        "go" -> Lang(slashes, cBlock, GO)
        "c", "h", "cpp", "c++", "java", "kotlin", "kt", "cs", "csharp", "swift" -> Lang(slashes, cBlock, C)
        "sh", "bash", "zsh", "shell", "console" -> Lang(listOf("#"), null, SHELL)
        "sql" -> Lang(listOf("--"), cBlock, SQL)
        "toml", "yaml", "yml", "ini" -> Lang(listOf("#"), null, DATA)
        "json", "jsonc" -> Lang(slashes, null, DATA)
        else -> Lang(emptyList(), null, emptySet())
    }
}

internal val Keyword = SpanStyle(color = Mauve, fontWeight = FontWeight.Bold)
internal val StringLit = SpanStyle(color = Green)
internal val NumberLit = SpanStyle(color = Yellow)
internal val TypeName = SpanStyle(color = Lavender)
internal val Call = SpanStyle(color = Subtext0, fontWeight = FontWeight.Bold)
internal val Comment = SpanStyle(color = Overlay0, fontStyle = FontStyle.Italic)
internal val Plain = SpanStyle(color = MdText)

/** As linhas de um bloco realçadas; um comentário de bloco pode passar de uma linha para a outra. */
internal fun highlight(name: String, lines: List<String>): AnnotatedString = buildAnnotatedString {
    val lang = lang(name)
    var inBlock = false
    lines.forEachIndexed { n, line ->
        if (n > 0) append('\n')
        if (lang.keywords.isEmpty()) { withStyle(Plain) { append(line) }; return@forEachIndexed }
        inBlock = highlightLine(this, lang, line, inBlock)
    }
}

private fun highlightLine(out: AnnotatedString.Builder, lang: Lang, line: String, startInBlock: Boolean): Boolean {
    var inBlock = startInBlock
    val sql = lang.keywords === SQL
    val rust = lang.keywords === RUST
    fun push(from: Int, to: Int, style: SpanStyle) = out.withStyle(style) { append(line.substring(from, to)) }
    fun isWord(c: Char) = c.isLetterOrDigit() || c == '_'
    var i = 0
    while (i < line.length) {
        if (inBlock) {
            val close = lang.block!!.second
            val j = line.indexOf(close, i)
            val end = if (j < 0) line.length else { inBlock = false; j + close.length }
            push(i, end, Comment); i = end; continue
        }
        val block = lang.block
        if (block != null && line.startsWith(block.first, i)) {
            inBlock = true; push(i, i + block.first.length, Comment); i += block.first.length; continue
        }
        // `#` só abre comentário no começo da linha ou depois de espaço (no shell, `$#` não é).
        if (lang.line.any { line.startsWith(it, i) && (it != "#" || i == 0 || line[i - 1].isWhitespace()) }) {
            push(i, line.length, Comment); break
        }
        val c = line[i]
        if (c == '"' || c == '\'' || c == '`') {
            var j = i + 1
            while (j < line.length && line[j] != c) j += if (line[j] == '\\') 2 else 1
            val end = minOf(j + 1, line.length)
            // Em Rust, `'a` é um lifetime, não uma string.
            val charLiteral = j < line.length && (j == i + 2 || line.getOrNull(i + 1) == '\\')
            if (!(c == '\'' && rust && !charLiteral)) { push(i, end, StringLit); i = end; continue }
        }
        if (c.isDigit() && !(i > 0 && isWord(line[i - 1]))) {
            var end = i
            while (end < line.length && (line[end].isLetterOrDigit() || line[end] == '.' || line[end] == '_')) end++
            push(i, end, NumberLit); i = end; continue
        }
        if (isWord(c)) {
            var end = i
            while (end < line.length && isWord(line[end])) end++
            val word = line.substring(i, end)
            val next = line.getOrNull(end)
            val style = when {
                (if (sql) word.lowercase() else word) in lang.keywords -> Keyword
                word[0].isUpperCase() -> TypeName
                next == '(' || (rust && next == '!') -> Call
                else -> Plain
            }
            push(i, end, style); i = end; continue
        }
        push(i, i + 1, Plain); i++
    }
    return inBlock
}
