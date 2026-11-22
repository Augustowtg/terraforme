package com.terraforme.remote.ui.screens

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import com.terraforme.remote.ui.theme.DarkBackground
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Code
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Terminal
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.terraforme.remote.data.Instance
import com.terraforme.remote.data.Tab
import com.terraforme.remote.ui.theme.DarkCardBorder
import com.terraforme.remote.ui.theme.DarkSurface
import com.terraforme.remote.ui.theme.DarkSurfaceVariant
import com.terraforme.remote.ui.theme.ErrorRed
import com.terraforme.remote.ui.theme.PurpleAccent
import com.terraforme.remote.ui.theme.RunningBlue
import com.terraforme.remote.ui.theme.SuccessGreen
import com.terraforme.remote.ui.theme.TextMuted
import com.terraforme.remote.ui.theme.TextPrimary
import com.terraforme.remote.ui.theme.TextSecondary
import com.terraforme.remote.ui.viewmodel.TerraUiState

@Composable
fun MonitorScreen(
    state: TerraUiState,
    onRefresh: () -> Unit,
    onOpen: (Instance) -> Unit
) {
    Column(modifier = Modifier.fillMaxSize()) {

        if (state.isLoading || state.isDiscovering) {
            LinearProgressIndicator(
                modifier = Modifier.fillMaxWidth(),
                color = RunningBlue
            )
        }

        LazyColumn(
            modifier = Modifier.fillMaxSize(),
            contentPadding = PaddingValues(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp)
        ) {
            // Header com Resumo
            item {
                StatusOverviewHeader(
                    isConnected = state.isConnected,
                    instanceCount = state.instances.size,
                    runningCount = state.instances.count { it.status.isRunning },
                    lastUpdated = state.lastUpdated,
                    onRefresh = onRefresh
                )
            }

            state.usage?.let { u -> item { UsageCard(u) } }

            if (!state.isConnected) {
                item {
                    DisconnectedCard(serverUrl = state.serverUrl, error = state.errorMessage)
                }
            } else if (state.instances.isEmpty()) {
                item {
                    EmptyInstancesCard()
                }
            } else {
                items(state.instances, key = { it.id }) { instance ->
                    InstanceCard(instance = instance, onClick = { onOpen(instance) })
                }
            }
        }
    }
}

@Composable
fun StatusOverviewHeader(
    isConnected: Boolean,
    instanceCount: Int,
    runningCount: Int,
    lastUpdated: String?,
    onRefresh: () -> Unit
) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = DarkSurface),
        shape = RoundedCornerShape(16.dp),
        border = androidx.compose.foundation.BorderStroke(1.dp, DarkCardBorder)
    ) {
        Column(modifier = Modifier.padding(16.dp)) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    val dotColor = if (isConnected) SuccessGreen else ErrorRed
                    Box(
                        modifier = Modifier
                            .size(10.dp)
                            .background(dotColor, CircleShape)
                    )
                    Spacer(modifier = Modifier.width(8.dp))
                    Text(
                        text = if (isConnected) "ONLINE" else "OFFLINE",
                        color = dotColor,
                        fontWeight = FontWeight.Bold,
                        fontSize = 12.sp
                    )
                }

                Row(verticalAlignment = Alignment.CenterVertically) {
                    if (lastUpdated != null) {
                        Text(
                            text = "Atualizado às $lastUpdated",
                            color = TextMuted,
                            fontSize = 11.sp
                        )
                    }
                    IconButton(onClick = onRefresh, modifier = Modifier.size(28.dp)) {
                        Icon(
                            imageVector = Icons.Default.Refresh,
                            contentDescription = "Atualizar",
                            tint = TextSecondary,
                            modifier = Modifier.size(16.dp)
                        )
                    }
                }
            }

            Spacer(modifier = Modifier.height(12.dp))

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween
            ) {
                StatPill(title = "Janelas do Terra", value = "$instanceCount", tint = TextPrimary)
                StatPill(
                    title = "Agentes Rodando",
                    value = "$runningCount",
                    tint = if (runningCount > 0) RunningBlue else TextSecondary
                )
            }
        }
    }
}

@Composable
fun StatPill(title: String, value: String, tint: Color) {
    Surface(
        color = DarkSurfaceVariant,
        shape = RoundedCornerShape(12.dp)
    ) {
        Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
            Text(text = title, color = TextMuted, fontSize = 11.sp)
            Text(text = value, color = tint, fontWeight = FontWeight.Bold, fontSize = 18.sp)
        }
    }
}

@Composable
fun InstanceCard(instance: Instance, onClick: () -> Unit) {
    val isRunning = instance.status.isRunning

    val transition = rememberInfiniteTransition(label = "pulse")
    val pulseAlpha by transition.animateFloat(
        initialValue = 0.4f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(
            animation = tween(800, easing = FastOutSlowInEasing),
            repeatMode = RepeatMode.Reverse
        ),
        label = "pulseAlpha"
    )

    Card(
        modifier = Modifier.fillMaxWidth().clickable(onClick = onClick),
        colors = CardDefaults.cardColors(containerColor = DarkSurface),
        shape = RoundedCornerShape(16.dp),
        border = androidx.compose.foundation.BorderStroke(
            1.dp,
            if (isRunning) RunningBlue else DarkCardBorder
        )
    ) {
        Column(modifier = Modifier.padding(16.dp)) {
            // Cabeçalho da Instância
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(
                        imageVector = Icons.Default.Computer,
                        contentDescription = null,
                        tint = PurpleAccent,
                        modifier = Modifier.size(20.dp)
                    )
                    Spacer(modifier = Modifier.width(8.dp))
                    Text(
                        text = instance.workspace,
                        color = TextPrimary,
                        fontWeight = FontWeight.Bold,
                        fontSize = 16.sp
                    )
                }

                // Badge de Estado
                Surface(
                    color = if (isRunning) RunningBlue.copy(alpha = 0.2f) else SuccessGreen.copy(alpha = 0.2f),
                    shape = RoundedCornerShape(20.dp)
                ) {
                    Row(
                        modifier = Modifier.padding(horizontal = 10.dp, vertical = 4.dp),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Box(
                            modifier = Modifier
                                .size(8.dp)
                                .then(if (isRunning) Modifier.alpha(pulseAlpha) else Modifier)
                                .background(if (isRunning) RunningBlue else SuccessGreen, CircleShape)
                        )
                        Spacer(modifier = Modifier.width(6.dp))
                        Text(
                            text = if (isRunning) "RODANDO" else "PARADO",
                            color = if (isRunning) RunningBlue else SuccessGreen,
                            fontWeight = FontWeight.Bold,
                            fontSize = 11.sp
                        )
                    }
                }
            }

            Spacer(modifier = Modifier.height(6.dp))

            Text(
                text = instance.cwd,
                color = TextMuted,
                fontSize = 12.sp,
                fontFamily = FontFamily.Monospace
            )

            Spacer(modifier = Modifier.height(12.dp))

            // Caixa do que está sendo feito
            val taskText = instance.status.task
            Surface(
                modifier = Modifier.fillMaxWidth(),
                color = DarkSurfaceVariant,
                shape = RoundedCornerShape(10.dp)
            ) {
                Column(modifier = Modifier.padding(12.dp)) {
                    Text(
                        text = "O QUE ESTÁ SENDO FEITO:",
                        color = TextMuted,
                        fontWeight = FontWeight.Bold,
                        fontSize = 10.sp
                    )
                    Spacer(modifier = Modifier.height(4.dp))
                    Text(
                        text = taskText ?: "Aguardando próxima instrução...",
                        color = if (taskText != null) TextPrimary else TextSecondary,
                        fontSize = 13.sp,
                        fontWeight = if (taskText != null) FontWeight.Medium else FontWeight.Normal,
                        fontFamily = FontFamily.Monospace
                    )
                }
            }

            // Abas / Chats
            if (instance.status.tabs.isNotEmpty()) {
                Spacer(modifier = Modifier.height(12.dp))
                Text(
                    text = "Abas / Agentes:",
                    color = TextMuted,
                    fontSize = 11.sp,
                    fontWeight = FontWeight.SemiBold
                )
                Spacer(modifier = Modifier.height(6.dp))
                instance.status.tabs.forEach { tab ->
                    TabRow(tab = tab)
                    Spacer(modifier = Modifier.height(4.dp))
                }
            }
        }
    }
}

@Composable
fun TabRow(tab: Tab) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(DarkBackground.copy(alpha = 0.5f), RoundedCornerShape(8.dp))
            .padding(horizontal = 10.dp, vertical = 6.dp),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(
                imageVector = Icons.Default.Terminal,
                contentDescription = null,
                tint = TextSecondary,
                modifier = Modifier.size(14.dp)
            )
            Spacer(modifier = Modifier.width(6.dp))
            Text(
                text = tab.title + (tab.name?.let { " ($it)" } ?: ""),
                color = TextPrimary,
                fontSize = 12.sp,
                fontWeight = FontWeight.Medium
            )
        }

        if (tab.task != null) {
            Text(
                text = tab.task,
                color = RunningBlue,
                fontSize = 11.sp,
                fontFamily = FontFamily.Monospace
            )
        } else {
            Text(text = "ocioso", color = TextMuted, fontSize = 11.sp)
        }
    }
}

@Composable
fun DisconnectedCard(serverUrl: String, error: String?) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = DarkSurface),
        shape = RoundedCornerShape(16.dp),
        border = androidx.compose.foundation.BorderStroke(1.dp, ErrorRed.copy(alpha = 0.5f))
    ) {
        Column(modifier = Modifier.padding(20.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(
                    imageVector = Icons.Default.Info,
                    contentDescription = null,
                    tint = ErrorRed,
                    modifier = Modifier.size(24.dp)
                )
                Spacer(modifier = Modifier.width(12.dp))
                Text(
                    text = "Servidor Offline",
                    color = ErrorRed,
                    fontWeight = FontWeight.Bold,
                    fontSize = 16.sp
                )
            }
            Spacer(modifier = Modifier.height(10.dp))
            Text(
                text = "Não foi possível conectar a $serverUrl.\n\n" +
                        "1. Abra o terra no PC: o servidor abre sozinho na rede (porta 8765).\n\n" +
                        "2. Deixe o celular no mesmo Wi-Fi e toque em Procurar na rede, nas configurações.\n\n" +
                        "3. Via cabo USB: adb reverse tcp:8765 tcp:8765 e use localhost.",
                color = TextSecondary,
                fontSize = 13.sp,
                lineHeight = 18.sp
            )
        }
    }
}

@Composable
fun EmptyInstancesCard() {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = DarkSurface),
        shape = RoundedCornerShape(16.dp)
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(32.dp),
            horizontalAlignment = Alignment.CenterHorizontally
        ) {
            Icon(
                imageVector = Icons.Default.Computer,
                contentDescription = null,
                tint = TextMuted,
                modifier = Modifier.size(48.dp)
            )
            Spacer(modifier = Modifier.height(12.dp))
            Text(
                text = "Nenhuma janela do Terra ativa",
                color = TextSecondary,
                fontWeight = FontWeight.Medium,
                fontSize = 15.sp
            )
            Spacer(modifier = Modifier.height(4.dp))
            Text(
                text = "Abra o terra em algum diretório para monitorá-lo aqui.",
                color = TextMuted,
                fontSize = 12.sp
            )
        }
    }
}
