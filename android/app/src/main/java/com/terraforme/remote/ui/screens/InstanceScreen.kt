package com.terraforme.remote.ui.screens

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.terraforme.remote.data.ChatMessage
import com.terraforme.remote.data.Instance
import com.terraforme.remote.ui.theme.ErrorRed
import com.terraforme.remote.ui.theme.RunningBlue
import com.terraforme.remote.ui.theme.DarkBackground
import com.terraforme.remote.ui.theme.DarkCardBorder
import com.terraforme.remote.ui.theme.DarkSurface
import com.terraforme.remote.ui.theme.Primary
import com.terraforme.remote.ui.theme.TextMuted
import com.terraforme.remote.ui.theme.TextPrimary
import com.terraforme.remote.ui.theme.TextSecondary

/** Uma instância do terra: as abas, o histórico do chat da aba escolhida e o input que digita nela, no PC. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun InstanceScreen(
    instance: Instance?,
    isSending: Boolean,
    sendResult: String?,
    history: List<ChatMessage>,
    onSelectTab: (String) -> Unit,
    onSend: (tab: String, text: String) -> Unit,
    onBack: () -> Unit
) {
    val tabs = instance?.status?.tabs.orEmpty()
    var selected by remember { mutableStateOf<String?>(null) }
    val tab = tabs.find { it.id == selected } ?: tabs.firstOrNull()
    var text by remember { mutableStateOf("") }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(instance?.workspace ?: "Instância fechada", color = TextPrimary, fontWeight = FontWeight.Bold) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Voltar", tint = TextPrimary)
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = DarkSurface)
            )
        },
        containerColor = DarkBackground
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .imePadding()
                .padding(16.dp)
        ) {
            if (instance == null) {
                Text("Este terra foi fechado.", color = TextSecondary)
                return@Column
            }
            Text(instance.cwd, color = TextMuted, fontSize = 12.sp, fontFamily = FontFamily.Monospace)
            Spacer(Modifier.height(12.dp))

            // As abas do terra, como na barra dele; só com mais de uma.
            if (tabs.size > 1) {
                Row(
                    modifier = Modifier.horizontalScroll(rememberScrollState()),
                    horizontalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    tabs.forEach { t ->
                        FilterChip(
                            selected = t.id == tab?.id,
                            onClick = { selected = t.id; onSelectTab(t.id) },
                            label = { Text(t.name ?: t.title) },
                            colors = FilterChipDefaults.filterChipColors(
                                selectedContainerColor = Primary,
                                selectedLabelColor = DarkSurface,
                                labelColor = TextSecondary
                            )
                        )
                    }
                }
                Spacer(Modifier.height(12.dp))
            }

            // O chat da aba, rolando para a última mensagem quando chega outra.
            val list = rememberLazyListState()
            LaunchedEffect(history.size) { if (history.isNotEmpty()) list.scrollToItem(history.lastIndex) }
            LazyColumn(
                state = list,
                modifier = Modifier.weight(1f).fillMaxWidth(),
                verticalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                items(history) { MessageLine(it) }
            }

            Text(
                text = tab?.task ?: "Aguardando próxima instrução...",
                color = if (tab?.task != null) RunningBlue else TextMuted,
                fontFamily = FontFamily.Monospace,
                fontSize = 12.sp
            )
            Spacer(Modifier.height(8.dp))

            sendResult?.let {
                Text(it, color = TextSecondary, fontSize = 12.sp)
                Spacer(Modifier.height(8.dp))
            }

            Row(verticalAlignment = Alignment.CenterVertically) {
                OutlinedTextField(
                    value = text,
                    onValueChange = { text = it },
                    placeholder = { Text("Digite no terra...", color = TextMuted) },
                    enabled = tab != null && !isSending,
                    colors = OutlinedTextFieldDefaults.colors(
                        focusedBorderColor = Primary,
                        unfocusedBorderColor = DarkCardBorder,
                        focusedTextColor = TextPrimary,
                        unfocusedTextColor = TextPrimary
                    ),
                    shape = RoundedCornerShape(12.dp),
                    modifier = Modifier.weight(1f)
                )
                IconButton(
                    onClick = {
                        if (tab != null && text.isNotBlank()) {
                            onSend(tab.id, text)
                            text = ""
                        }
                    },
                    enabled = tab != null && text.isNotBlank() && !isSending
                ) {
                    Icon(Icons.AutoMirrored.Filled.Send, contentDescription = "Enviar", tint = Primary)
                }
            }
        }
    }
}

/** Uma mensagem do chat, como no terra: o usuário em negrito, as ferramentas em monoespaçado. */
@Composable
private fun MessageLine(m: ChatMessage) {
    when (m.role) {
        "user" -> Text("› ${m.content}", color = TextPrimary, fontWeight = FontWeight.Bold, fontSize = 14.sp)
        "assistant" -> Text(m.content, color = TextSecondary, fontSize = 14.sp)
        "tool" -> Text("● ${m.tool.orEmpty()} ${m.content}", color = Primary, fontFamily = FontFamily.Monospace, fontSize = 12.sp)
        "error", "tool_error" -> Text(m.content, color = ErrorRed, fontFamily = FontFamily.Monospace, fontSize = 12.sp)
        else -> Text(m.content, color = TextMuted, fontFamily = FontFamily.Monospace, fontSize = 12.sp, maxLines = 4)
    }
}
