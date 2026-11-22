package com.terraforme.remote

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.viewModels
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Build
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material.icons.filled.History
import androidx.compose.material.icons.filled.Send
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.terraforme.remote.ui.screens.InstanceScreen
import com.terraforme.remote.ui.screens.MonitorScreen
import com.terraforme.remote.ui.screens.RunDetailScreen
import com.terraforme.remote.ui.screens.RunsScreen
import com.terraforme.remote.ui.theme.DarkBackground
import com.terraforme.remote.ui.theme.DarkCardBorder
import com.terraforme.remote.ui.theme.DarkSurface
import com.terraforme.remote.ui.theme.DarkSurfaceVariant
import com.terraforme.remote.ui.theme.ErrorRed
import com.terraforme.remote.ui.theme.Primary
import com.terraforme.remote.ui.theme.SuccessGreen
import com.terraforme.remote.ui.theme.TerraRemoteTheme
import com.terraforme.remote.ui.theme.TextMuted
import com.terraforme.remote.ui.theme.TextPrimary
import com.terraforme.remote.ui.theme.TextSecondary
import com.terraforme.remote.ui.viewmodel.TerraViewModel

class MainActivity : ComponentActivity() {

    private val viewModel: TerraViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            TerraRemoteTheme {
                MainAppScreen(viewModel = viewModel)
            }
        }
    }

    override fun onResume() {
        super.onResume()
        viewModel.startPolling()
        viewModel.refreshAll()
    }

    override fun onPause() {
        super.onPause()
        viewModel.stopPolling()
    }
}

enum class ScreenTab(val title: String, val icon: ImageVector) {
    INSTANCES("Instâncias", Icons.Default.Computer),
    RUNS("Execuções", Icons.Default.History)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MainAppScreen(viewModel: TerraViewModel) {
    val state by viewModel.uiState.collectAsState()
    var selectedTab by remember { mutableIntStateOf(0) }
    var showSettingsDialog by remember { mutableStateOf(false) }

    // Se estiver visualizando detalhes de um run
    if (state.selectedRunDetail != null || state.isLoadingDetail) {
        RunDetailScreen(
            detail = state.selectedRunDetail,
            isLoading = state.isLoadingDetail,
            onBack = { viewModel.clearRunDetail() }
        )
        return
    }

    if (state.openInstanceId != null) {
        BackHandler { viewModel.openInstance(null) }
        InstanceScreen(
            instance = state.instances.find { it.id == state.openInstanceId },
            isSending = state.isSending,
            sendResult = state.sendResult,
            onSend = { tab, text -> viewModel.sendInput(tab, text) },
            onBack = { viewModel.openInstance(null) }
        )
        return
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(
                            text = "Terra Remote",
                            fontWeight = FontWeight.Bold,
                            color = TextPrimary,
                            fontSize = 19.sp
                        )
                        Spacer(modifier = Modifier.width(10.dp))
                        // Pill de conexão
                        Surface(
                            color = if (state.isConnected) SuccessGreen.copy(alpha = 0.2f) else ErrorRed.copy(alpha = 0.2f),
                            shape = RoundedCornerShape(12.dp),
                            modifier = Modifier.clickable { showSettingsDialog = true }
                        ) {
                            Row(
                                modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
                                verticalAlignment = Alignment.CenterVertically
                            ) {
                                Box(
                                    modifier = Modifier
                                        .size(6.dp)
                                        .background(
                                            if (state.isConnected) SuccessGreen else ErrorRed,
                                            CircleShape
                                        )
                                )
                                Spacer(modifier = Modifier.width(5.dp))
                                Text(
                                    text = if (state.isConnected) "Conectado" else "Desconectado",
                                    color = if (state.isConnected) SuccessGreen else ErrorRed,
                                    fontSize = 11.sp,
                                    fontWeight = FontWeight.Bold
                                )
                            }
                        }
                    }
                },
                actions = {
                    IconButton(onClick = { showSettingsDialog = true }) {
                        Icon(
                            imageVector = Icons.Default.Settings,
                            contentDescription = "Configurar Servidor",
                            tint = TextSecondary
                        )
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = DarkSurface)
            )
        },
        bottomBar = {
            NavigationBar(
                containerColor = DarkSurface,
                tonalElevation = 8.dp
            ) {
                ScreenTab.values().forEachIndexed { index, tab ->
                    NavigationBarItem(
                        selected = selectedTab == index,
                        onClick = {
                            selectedTab = index
                            if (index == 1) viewModel.fetchRuns()
                        },
                        icon = { Icon(imageVector = tab.icon, contentDescription = tab.title) },
                        label = { Text(text = tab.title, fontSize = 12.sp) },
                        colors = NavigationBarItemDefaults.colors(
                            selectedIconColor = DarkSurface,
                            selectedTextColor = Primary,
                            indicatorColor = Primary,
                            unselectedIconColor = TextMuted,
                            unselectedTextColor = TextMuted
                        )
                    )
                }
            }
        },
        containerColor = DarkBackground
    ) { padding ->
        Box(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
        ) {
            when (selectedTab) {
                0 -> MonitorScreen(
                    state = state,
                    onRefresh = { viewModel.refreshAll() },
                    onOpen = { viewModel.openInstance(it.id) }
                )
                1 -> RunsScreen(
                    runs = state.runs,
                    onSelectRun = { runId -> viewModel.loadRunDetail(runId) },
                    onRefresh = { viewModel.fetchRuns() }
                )
            }
        }
    }

    if (showSettingsDialog) {
        ServerSettingsDialog(
            currentUrl = state.serverUrl,
            isDiscovering = state.isDiscovering,
            onDiscover = {
                viewModel.discover()
                showSettingsDialog = false
            },
            onDismiss = { showSettingsDialog = false },
            onSave = { newUrl ->
                viewModel.updateServerUrl(newUrl)
                showSettingsDialog = false
            }
        )
    }
}

@Composable
fun ServerSettingsDialog(
    currentUrl: String,
    isDiscovering: Boolean,
    onDiscover: () -> Unit,
    onDismiss: () -> Unit,
    onSave: (String) -> Unit
) {
    var urlText by remember { mutableStateOf(currentUrl) }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = {
            Text(
                text = "Endereço do Servidor Terra",
                color = TextPrimary,
                fontWeight = FontWeight.Bold,
                fontSize = 17.sp
            )
        },
        text = {
            Column {
                Text(
                    text = "Digite a URL onde o terra remote está rodando:",
                    color = TextSecondary,
                    fontSize = 13.sp
                )
                Spacer(modifier = Modifier.height(10.dp))
                OutlinedTextField(
                    value = urlText,
                    onValueChange = { urlText = it },
                    singleLine = true,
                    colors = OutlinedTextFieldDefaults.colors(
                        focusedBorderColor = Primary,
                        unfocusedBorderColor = DarkCardBorder,
                        focusedTextColor = TextPrimary,
                        unfocusedTextColor = TextPrimary
                    ),
                    shape = RoundedCornerShape(10.dp),
                    modifier = Modifier.fillMaxWidth()
                )

                Spacer(modifier = Modifier.height(12.dp))

                Text(
                    text = "Opções rápidas:",
                    color = TextMuted,
                    fontSize = 11.sp,
                    fontWeight = FontWeight.SemiBold
                )

                Spacer(modifier = Modifier.height(6.dp))

                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    Surface(
                        color = DarkSurfaceVariant,
                        shape = RoundedCornerShape(8.dp),
                        modifier = Modifier.clickable { urlText = "http://localhost:8765" }
                    ) {
                        Text(
                            text = "USB (localhost)",
                            color = TextPrimary,
                            fontSize = 11.sp,
                            modifier = Modifier.padding(horizontal = 8.dp, vertical = 6.dp)
                        )
                    }

                    Surface(
                        color = DarkSurfaceVariant,
                        shape = RoundedCornerShape(8.dp),
                        modifier = Modifier.clickable(enabled = !isDiscovering, onClick = onDiscover)
                    ) {
                        Text(
                            text = if (isDiscovering) "Procurando..." else "Procurar na rede",
                            color = TextPrimary,
                            fontSize = 11.sp,
                            modifier = Modifier.padding(horizontal = 8.dp, vertical = 6.dp)
                        )
                    }
                }

                Spacer(modifier = Modifier.height(12.dp))

                Text(
                    text = "Dica USB: com o celular plugado via cabo, rode no PC:\n" +
                            "adb reverse tcp:8765 tcp:8765",
                    color = TextMuted,
                    fontSize = 11.sp,
                    fontFamily = FontFamily.Monospace,
                    lineHeight = 15.sp
                )
            }
        },
        confirmButton = {
            Button(
                onClick = { onSave(urlText) },
                colors = ButtonDefaults.buttonColors(containerColor = Primary)
            ) {
                Text(text = "Salvar e Conectar", color = DarkSurface, fontWeight = FontWeight.Bold)
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text(text = "Cancelar", color = TextSecondary)
            }
        },
        containerColor = DarkSurface
    )
}
