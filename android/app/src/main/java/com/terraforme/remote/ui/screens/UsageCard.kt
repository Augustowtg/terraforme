package com.terraforme.remote.ui.screens

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.terraforme.remote.data.AgentUsage
import com.terraforme.remote.data.Usage
import com.terraforme.remote.ui.theme.*
import java.util.Locale

private fun tokens(n: Long) = when {
    n >= 1_000_000 -> String.format(Locale.US, "%.1fM", n / 1e6)
    n >= 1_000 -> String.format(Locale.US, "%.1fk", n / 1e3)
    else -> n.toString()
}

private fun usd(v: Double) = String.format(Locale.US, "$%.2f", v)

/** A aba Custo: o consumo de hoje do terra conectado. */
@Composable
fun CostScreen(usage: Usage?) {
    Column(Modifier.fillMaxSize().padding(16.dp)) {
        if (usage != null) UsageCard(usage)
        else Text("Sem dados de consumo ainda.", color = TextMuted, fontSize = 13.sp)
    }
}

@Composable
fun UsageCard(usage: Usage) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = DarkSurface),
        shape = RoundedCornerShape(16.dp),
        border = BorderStroke(1.dp, DarkCardBorder)
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text("Consumo de hoje", color = TextPrimary, fontWeight = FontWeight.Bold, fontSize = 16.sp)
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                Stat("Custo", usd(usage.totalCostUsd))
                Stat("Tokens", tokens(usage.totalTokens))
                Stat("Execuções", usage.totalRuns.toString())
                Stat("Rodando", usage.running.toString())
            }
            usage.agents.filter { it.runs > 0 || it.budget != null }.forEach { AgentRow(it) }
        }
    }
}

@Composable
private fun Stat(label: String, value: String) {
    Column {
        Text(value, color = Primary, fontWeight = FontWeight.Bold, fontSize = 18.sp)
        Text(label, color = TextMuted, fontSize = 11.sp)
    }
}

@Composable
private fun AgentRow(a: AgentUsage) {
    val color = when (a.alert) {
        "blocked" -> ErrorRed
        "warning" -> WarningYellow
        else -> Primary
    }
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            Text(a.agent, color = TextPrimary, fontSize = 13.sp, fontWeight = FontWeight.SemiBold)
            Text(
                "${a.runs}${a.budget?.let { "/$it" } ?: ""} · ${tokens(a.tokens)} · ${usd(a.costUsd)}",
                color = color, fontSize = 12.sp
            )
        }
        a.budget?.takeIf { it > 0 }?.let {
            LinearProgressIndicator(
                progress = { (a.runs.toFloat() / it).coerceIn(0f, 1f) },
                modifier = Modifier.fillMaxWidth(),
                color = color,
                trackColor = DarkSurfaceVariant
            )
        }
    }
}
