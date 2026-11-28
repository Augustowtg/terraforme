package com.terraforme.remote.ui.screens

import androidx.compose.ui.text.SpanStyle
import org.junit.Assert.assertEquals
import org.junit.Test

class MarkdownTest {
    @Test
    fun codeBlockKeepsLanguageEvenWhileStreaming() {
        assertEquals(
            listOf(
                Segment.Prose(listOf("oi")),
                Segment.Fence("js", listOf("const x = 1")),
                Segment.Prose(listOf("fim")),
                Segment.Fence("rust", listOf("fn")),
            ),
            segments("oi\n```js\nconst x = 1\n```\nfim\n```rust\nfn"),
        )
    }

    @Test
    fun jsGetsIdeColors() {
        val text = "const nome = soma(\"a\", 42); // c"
        val code = highlight("js", listOf(text))
        fun styleOf(word: String): SpanStyle =
            code.spanStyles.first { it.start == text.indexOf(word) }.item
        assertEquals(Keyword, styleOf("const"))
        assertEquals(Plain, styleOf("nome"))
        assertEquals(Call, styleOf("soma"))
        assertEquals(StringLit, styleOf("\"a\""))
        assertEquals(NumberLit, styleOf("42"))
        assertEquals(Comment, styleOf("// c"))
    }
}
