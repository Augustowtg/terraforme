package com.terraforme.remote.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
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
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ChevronRight
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.terraforme.remote.data.RunItem
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
import com.terraforme.remote.ui.theme.WarningYellow
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

@Composable
fun RunsScreen(
    runs: List<RunItem>,
    onSelectRun: (String) -> Unit,
    onRefresh: () -> Unit
) {
    Column(modifier = Modifier.fillMaxSize()) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 12.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically
        ) {
            Text(
                text = "Execuções dos Agentes (${runs.size})",
                color = TextPrimary,
                fontWeight = FontWeight.Bold,
                fontSize = 18.sp
            )
            IconButton(onClick = onRefresh) {
                Icon(
                    imageVector = Icons.Default.Refresh,
                    contentDescription = "Atualizar",
                    tint = TextSecondary
                )
            }
        }

        if (runs.isEmpty()) {
            Box(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(32.dp),
                contentAlignment = Alignment.Center
            ) {
                Text(
                    text = "Nenhuma execução registrada ainda.",
                    color = TextMuted,
                    fontSize = 14.sp
                )
            }
        } else {
            LazyColumn(
                modifier = Modifier.fillMaxSize(),
                contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp)
            ) {
                items(runs, key = { it.id }) { run ->
                    RunCard(run = run, onClick = { onSelectRun(run.id) })
                }
            }
        }
    }
}

@Composable
fun RunCard(run: RunItem, onClick: () -> Unit) {
    val stateColor = when (run.state.lowercase()) {
        "done" -> SuccessGreen
        "running" -> RunningBlue
        "failed" -> ErrorRed
        "queued" -> WarningYellow
        else -> TextMuted
    }

    val dateStr = run.started?.let {
        SimpleDateFormat("dd/MM HH:mm:ss", Locale.getDefault()).format(Date(it * 1000))
    } ?: "Aguardando"

    Card(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick),
        colors = CardDefaults.cardColors(containerColor = DarkSurface),
        shape = RoundedCornerShape(14.dp),
        border = androidx.compose.foundation.BorderStroke(1.dp, DarkCardBorder)
    ) {
        Column(modifier = Modifier.padding(14.dp)) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        text = run.agent,
                        color = TextPrimary,
                        fontWeight = FontWeight.Bold,
                        fontSize = 15.sp
                    )
                    Spacer(modifier = Modifier.width(8.dp))
                    Text(
                        text = run.id.take(8),
                        color = TextMuted,
                        fontFamily = FontFamily.Monospace,
                        fontSize = 12.sp
                    )
                }

                Surface(
                    color = stateColor.copy(alpha = 0.2f),
                    shape = RoundedCornerShape(12.dp)
                ) {
                    Text(
                        text = run.state.uppercase(),
                        color = stateColor,
                        fontWeight = FontWeight.Bold,
                        fontSize = 11.sp,
                        modifier = Modifier.padding(horizontal = 8.dp, vertical = 3.dp)
                    )
                }
            }

            Spacer(modifier = Modifier.height(6.dp))

            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    text = dateStr,
                    color = TextMuted,
                    fontSize = 12.sp
                )
                if (run.model != null) {
                    Text(
                        text = " · ${run.model}",
                        color = PurpleAccent,
                        fontSize = 12.sp
                    )
                }
            }

            if (run.summary != null) {
                Spacer(modifier = Modifier.height(8.dp))
                Text(
                    text = run.summary,
                    color = TextSecondary,
                    fontSize = 13.sp,
                    maxLines = 2
                )
            } else if (run.error != null) {
                Spacer(modifier = Modifier.height(8.dp))
                Text(
                    text = run.error,
                    color = ErrorRed,
                    fontSize = 12.sp,
                    fontFamily = FontFamily.Monospace,
                    maxLines = 2
                )
            }

            if (run.usage != null) {
                Spacer(modifier = Modifier.height(8.dp))
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text(
                        text = "${run.usage.input + run.usage.output} tokens · ${run.usage.turns} turnos",
                        color = TextMuted,
                        fontSize = 11.sp
                    )
                    Icon(
                        imageVector = Icons.Default.ChevronRight,
                        contentDescription = "Ver Detalhes",
                        tint = TextSecondary,
                        modifier = Modifier.size(16.dp)
                    )
                }
            }
        }
    }
}
