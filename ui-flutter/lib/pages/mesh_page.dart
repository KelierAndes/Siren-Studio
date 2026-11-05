import 'dart:io';

import 'package:flutter/material.dart';

import '../app_model.dart';
import '../models.dart';
import '../theme.dart';
import '../widgets.dart';

/// 组网 (desktop): decentralized mesh — one node receives, mixes and sends.
class MeshPage extends StatefulWidget {
  const MeshPage({super.key, required this.model});

  final AppModel model;

  @override
  State<MeshPage> createState() => _MeshPageState();
}

class _MeshPageState extends State<MeshPage> {
  late final TextEditingController _name =
      TextEditingController(text: defaultNodeName());
  String _source = 'tone';
  bool _output = true;
  late final TextEditingController _freq = TextEditingController(text: '440');

  static String defaultNodeName() {
    if (Platform.isWindows) {
      return Platform.environment['COMPUTERNAME'] ?? 'Siren Node';
    }
    return Platform.environment['HOSTNAME'] ?? 'Siren Node';
  }

  @override
  void dispose() {
    _name.dispose();
    _freq.dispose();
    super.dispose();
  }

  void _start() {
    final freq = double.tryParse(_freq.text.trim()) ?? 440;
    widget.model.startMesh(
      name: _name.text.trim().isEmpty ? 'Siren Node' : _name.text.trim(),
      source: _source,
      output: _output,
      freq: freq,
    );
  }

  @override
  Widget build(BuildContext context) {
    final model = widget.model;
    return AnimatedBuilder(
      animation: model,
      builder: (context, _) {
        final mesh = model.mesh;
        final running = mesh?.running == true;
        final children = <Widget>[];

        if (!running) {
          children.add(_buildConfig(model));
        } else {
          children.add(_buildLive(model, mesh!));
          children.add(const SizedBox(height: 18));
          children.add(_buildPeers(mesh));
          children.add(const SizedBox(height: 18));
          if (mesh.center.streams.isEmpty) {
            children.add(
              Panel(
                child: EmptyState(
                  compact: true,
                  icon: Icons.lan_rounded,
                  title: '尚未发现其他节点',
                  message: '在其他设备上打开 SirenStudio 并进入组网，节点会通过 mDNS 自动互相发现。',
                ),
              ),
            );
          } else {
            children.add(
              SectionLabel(
                '收到的音频',
                trailing: Text(
                  '${mesh.center.streams.length} 路',
                  style: AppText.mono.copyWith(
                    color: AppColors.textFaint,
                    fontSize: 11,
                  ),
                ),
              ),
            );
            children.addAll([
              for (final stream in mesh.center.streams) ...[
                StreamCard(
                  key: ValueKey(stream.id),
                  model: model,
                  stream: stream,
                ),
                const SizedBox(height: 12),
              ],
            ]);
          }
          children.add(const SizedBox(height: 6));
          children.add(
            Align(
              alignment: Alignment.centerRight,
              child: OutlinedButton.icon(
                onPressed: mesh.stopping ? null : model.stopMesh,
                icon: const Icon(Icons.stop_rounded, size: 18),
                label: Text(mesh.stopping ? '离开中…' : '离开组网'),
                style: OutlinedButton.styleFrom(
                  foregroundColor: AppColors.rose,
                  side: BorderSide(
                    color: AppColors.rose.withValues(
                      alpha: mesh.stopping ? 0.2 : 0.45,
                    ),
                  ),
                ),
              ),
            ),
          );
        }

        return ListView(
          padding: const EdgeInsets.fromLTRB(28, 22, 28, 28),
          children: children,
        );
      },
    );
  }

  Widget _buildConfig(AppModel model) {
    final busy = model.busyMesh;
    return Panel(
      accent: AppColors.accent,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          PanelHeader(
            icon: Icons.lan_rounded,
            title: '加入组网',
            subtitle: 'Wi-Fi 与蓝牙自动同时工作：无需选择接入方式，任一路断开另一条自动接管',
          ),
          const SizedBox(height: 20),
          Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Expanded(
                flex: 4,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    const Text('节点名称',
                        style: TextStyle(color: AppColors.textDim, fontSize: 12)),
                    const SizedBox(height: 8),
                    TextField(
                      controller: _name,
                      style: const TextStyle(color: AppColors.text, fontSize: 13),
                      decoration: _inputDecoration(),
                    ),
                  ],
                ),
              ),
              const SizedBox(width: 20),
              Expanded(
                flex: 4,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    const Text('本机音频来源',
                        style: TextStyle(color: AppColors.textDim, fontSize: 12)),
                    const SizedBox(height: 8),
                    SegmentedButton<String>(
                      segments: const [
                        ButtonSegment(
                          value: 'tone',
                          icon: Icon(Icons.waves_rounded, size: 16),
                          label: Text('测试音'),
                        ),
                        ButtonSegment(
                          value: 'loopback',
                          icon: Icon(Icons.desktop_windows_rounded, size: 16),
                          label: Text('系统内录'),
                        ),
                        ButtonSegment(
                          value: 'microphone',
                          icon: Icon(Icons.mic_rounded, size: 16),
                          label: Text('麦克风'),
                        ),
                      ],
                      selected: {_source},
                      onSelectionChanged: (selection) =>
                          setState(() => _source = selection.first),
                    ),
                  ],
                ),
              ),
              const SizedBox(width: 20),
              Expanded(
                flex: 3,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    const Text('本地播放收到的音频',
                        style: TextStyle(color: AppColors.textDim, fontSize: 12)),
                    const SizedBox(height: 8),
                    Row(
                      children: [
                        Text(
                          _output ? '播放' : '静默',
                          style: const TextStyle(
                            color: AppColors.textDim,
                            fontSize: 12,
                          ),
                        ),
                        const Spacer(),
                        Switch(
                          value: _output,
                          onChanged: (value) =>
                              setState(() => _output = value),
                          activeThumbColor: AppColors.accent,
                        ),
                      ],
                    ),
                  ],
                ),
              ),
            ],
          ),
          if (_source == 'tone') ...[
            const SizedBox(height: 16),
            SizedBox(
              width: 220,
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const Text('测试音频率 (Hz)',
                      style: TextStyle(color: AppColors.textDim, fontSize: 12)),
                  const SizedBox(height: 8),
                  TextField(
                    controller: _freq,
                    keyboardType: TextInputType.number,
                    style: const TextStyle(color: AppColors.text, fontSize: 13),
                    decoration: _inputDecoration(),
                  ),
                ],
              ),
            ),
          ],
          const SizedBox(height: 22),
          Align(
            alignment: Alignment.centerRight,
            child: FilledButton.icon(
              onPressed: busy ? null : _start,
              icon: busy
                  ? const AppSpinner(size: 16, color: AppColors.ink)
                  : const Icon(Icons.play_arrow_rounded, size: 20),
              label: Text(busy ? '启动中…' : '加入组网'),
            ),
          ),
        ],
      ),
    );
  }

  InputDecoration _inputDecoration() {
    return InputDecoration(
      filled: true,
      fillColor: AppColors.bg2,
      contentPadding:
          const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
      enabledBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(12),
        borderSide: const BorderSide(color: AppColors.border),
      ),
      focusedBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(12),
        borderSide: BorderSide(color: AppColors.accent.withValues(alpha: 0.6)),
      ),
    );
  }

  Widget _buildLive(AppModel model, MeshState mesh) {
    final master = mesh.center.master;
    return Panel(
      accent: AppColors.accent,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          PanelHeader(
            icon: Icons.graphic_eq_rounded,
            title: '主输出',
            subtitle:
                '${modeLabel(mesh.center.info.mode)} · 已运行 ${formatUptime(mesh.seconds)} · ${mesh.bind}',
            trailing: StatusPill(
              label: '${master.activeStreams} 路混音',
              color: AppColors.accent2,
              dot: false,
            ),
          ),
          const SizedBox(height: 20),
          Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Expanded(
                flex: 5,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      crossAxisAlignment: CrossAxisAlignment.baseline,
                      textBaseline: TextBaseline.alphabetic,
                      children: [
                        Text(
                          formatDb(master.rmsDb),
                          style: AppText.mono.copyWith(
                            fontSize: 42,
                            height: 1,
                            fontWeight: FontWeight.w700,
                            letterSpacing: -2,
                            color: AppColors.text,
                          ),
                        ),
                        const SizedBox(width: 8),
                        const Text('dBFS',
                            style:
                                TextStyle(color: AppColors.textDim, fontSize: 12)),
                      ],
                    ),
                    const SizedBox(height: 14),
                    LevelMeter(
                      rmsDb: master.rmsDb,
                      peakDb: master.peakDb,
                      peakHoldDb: master.peakHoldDb,
                      height: 14,
                      showValue: false,
                    ),
                    const SizedBox(height: 12),
                    Wrap(
                      spacing: 8,
                      runSpacing: 8,
                      children: [
                        StatChip(
                          label: '峰值保持',
                          value: '${formatDb(master.peakHoldDb)} dB',
                          color: AppColors.amber,
                        ),
                        StatChip(
                          label: '收包',
                          value: formatCount(mesh.center.receiver.datagrams),
                        ),
                        StatChip(
                          label: '发出',
                          value: formatCount(mesh.senderPackets),
                          color: AppColors.accent2,
                        ),
                        StatChip(
                          label: '上行',
                          value:
                              '${mesh.senderKbps.round()} kbps',
                          color: AppColors.accent2,
                        ),
                      ],
                    ),
                  ],
                ),
              ),
              const SizedBox(width: 28),
              Expanded(
                flex: 4,
                child: Column(
                  children: [
                    MeshSliderRow(
                      label: '缓冲目标',
                      value: mesh.center.targetMs,
                      min: 15,
                      max: 100,
                      display: '${mesh.center.targetMs.round()} ms',
                      onChanged: model.setTargetMs,
                    ),
                    const SizedBox(height: 10),
                    MeshSliderRow(
                      label: '主增益',
                      value: mesh.center.masterGainDb,
                      min: -24,
                      max: 12,
                      display:
                          '${mesh.center.masterGainDb >= 0 ? '+' : ''}${mesh.center.masterGainDb.toStringAsFixed(1)} dB',
                      onChanged: model.setMasterGainDb,
                    ),
                  ],
                ),
              ),
            ],
          ),
        ],
      ),
    );
  }

  Widget _buildPeers(MeshState mesh) {
    return Panel(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          PanelHeader(
            icon: Icons.lan_rounded,
            title: '已连接节点',
            subtitle: 'Wi-Fi 与蓝牙两条通路同时工作，按序号自动去重',
            trailing: StatusPill(
              label: mesh.bt.label,
              color: mesh.bt.links > 0
                  ? AppColors.accent2
                  : AppColors.textFaint,
              dot: false,
            ),
          ),
          const SizedBox(height: 14),
          if (mesh.peers.isEmpty)
            const Text(
              '正在扫描局域网…',
              style: TextStyle(color: AppColors.textFaint, fontSize: 12),
            )
          else
            Wrap(
              spacing: 10,
              runSpacing: 10,
              children: [
                for (final peer in mesh.peers)
                  Container(
                    padding:
                        const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
                    decoration: BoxDecoration(
                      color: AppColors.bg2,
                      borderRadius: BorderRadius.circular(12),
                      border: Border.all(color: AppColors.borderSoft),
                    ),
                    child: Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        const Icon(Icons.computer_rounded,
                            size: 15, color: AppColors.accent),
                        const SizedBox(width: 8),
                        Text(
                          peer.displayName,
                          style: const TextStyle(
                            color: AppColors.text,
                            fontSize: 12.5,
                            fontWeight: FontWeight.w600,
                          ),
                        ),
                        const SizedBox(width: 8),
                        Text(
                          // "蓝牙 <mac>" 是已知的 RFCOMM 地址（能力），不是当前载波：
                          // 双栈并行发送，两端各自看到的入向可能不同，写"BT 连接"会被读成链路只走蓝牙。
                          peer.bt == null
                              ? peer.endpoint
                              : '${peer.endpoint} · 蓝牙 ${peer.bt}',
                          style: AppText.mono.copyWith(
                            color: AppColors.textFaint,
                            fontSize: 11,
                          ),
                        ),
                      ],
                    ),
                  ),
              ],
            ),
        ],
      ),
    );
  }
}

class MeshSliderRow extends StatelessWidget {
  const MeshSliderRow({
    super.key,
    required this.label,
    required this.value,
    required this.min,
    required this.max,
    required this.display,
    required this.onChanged,
  });

  final String label;
  final double value;
  final double min;
  final double max;
  final String display;
  final ValueChanged<double> onChanged;

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Row(
          children: [
            Text(
              label,
              style: const TextStyle(
                color: AppColors.textDim,
                fontSize: 11,
                fontWeight: FontWeight.w600,
              ),
            ),
            const Spacer(),
            Text(
              display,
              style: AppText.mono.copyWith(
                color: AppColors.accent,
                fontSize: 11,
                fontWeight: FontWeight.w700,
              ),
            ),
          ],
        ),
        Slider(
          min: min,
          max: max,
          value: value.clamp(min, max),
          onChanged: onChanged,
        ),
      ],
    );
  }
}
