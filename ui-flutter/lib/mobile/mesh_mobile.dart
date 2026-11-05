import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';

import '../app_model.dart';
import '../models.dart';
import '../theme.dart';
import '../widgets.dart';
import 'mobile_widgets.dart';

/// 组网 tab (mobile): decentralized mesh, one tap to join.
class MeshMobile extends StatefulWidget {
  const MeshMobile({super.key, required this.model});

  final AppModel model;

  @override
  State<MeshMobile> createState() => _MeshMobileState();
}

class _MeshMobileState extends State<MeshMobile> {
  late final TextEditingController _name = TextEditingController(
    text: Platform.environment['HOSTNAME'] ??
        Platform.environment['COMPUTERNAME'] ??
        'Siren Node',
  );
  late final TextEditingController _freq = TextEditingController(text: '440');
  String _source = 'tone';
  bool _output = true;

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

  void updateSource(String value) => setState(() => _source = value);

  void updateOutput(bool value) => setState(() => _output = value);

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: widget.model,
      builder: (context, _) {
        final model = widget.model;
        final mesh = model.mesh;
        final running = mesh?.running == true;
        return ListView(
          padding: const EdgeInsets.fromLTRB(18, 4, 18, 24),
          children: [
            if (model.lastError != null)
              MobileNotice(
                message: model.lastError!,
                color: AppColors.rose,
                onDismiss: model.clearError,
              ),
            if (!running)
              _ConfigCard(state: this)
            else ...[
              _LiveCard(model: model, mesh: mesh!),
              const SizedBox(height: 14),
              if (mesh.peers.isEmpty)
                const MobileEmpty(
                  icon: Icons.lan_rounded,
                  title: '正在扫描局域网…',
                  message: '其他设备加入组网后会自动出现在这里，无需任何配置。',
                )
              else
                Padding(
                  padding: const EdgeInsets.only(bottom: 10),
                  child: MobileStat(
                    label: '蓝牙通路',
                    value: mesh.bt.label,
                    color: mesh.bt.links > 0
                        ? AppColors.accent2
                        : AppColors.textDim,
                  ),
                ),
                ...mesh.peers.map(
                  (peer) => Padding(
                    padding: const EdgeInsets.only(bottom: 10),
                    child: MobileStat(
                      label: peer.displayName,
                      value: peer.bt == null
                          ? peer.endpoint
                          : '${peer.endpoint} · 蓝牙 ${peer.bt}',
                      color: AppColors.accent,
                    ),
                  ),
                ),
              const SizedBox(height: 8),
              ...mesh.center.streams.map(
                (stream) => Padding(
                  padding: const EdgeInsets.only(bottom: 12),
                  child: StreamTile(model: model, stream: stream),
                ),
              ),
              if (mesh.center.streams.isEmpty)
                const MobileEmpty(
                  icon: Icons.wifi_tethering_rounded,
                  title: '还没有收到音频',
                  message: '对端设备开始发送本机音频后会自动出现在这里。',
                ),
              const SizedBox(height: 6),
              MobileDangerButton(
                label: mesh.stopping ? '离开中…' : '离开组网',
                onPressed: mesh.stopping ? null : model.stopMesh,
              ),
            ],
          ],
        );
      },
    );
  }
}

class _ConfigCard extends StatelessWidget {
  const _ConfigCard({required this.state});

  final _MeshMobileState state;

  @override
  Widget build(BuildContext context) {
    final busy = state.widget.model.busyMesh;
    return MobileCard(
      icon: Icons.lan_rounded,
      title: '加入组网',
      subtitle: 'Wi-Fi 与蓝牙自动同时工作：无需选择接入方式，断了会无缝接管',
      children: [
        MobileTextField(
          controller: state._name,
          label: '节点名称',
        ),
        const SizedBox(height: 14),
        MobileSegmented<String>(
          label: '本机音频来源',
          value: state._source,
          items: const [
            (value: 'tone', label: '测试音', icon: Icons.waves_rounded),
            (
              value: 'loopback',
              label: '系统内录',
              icon: Icons.desktop_windows_rounded,
            ),
            (value: 'microphone', label: '麦克风', icon: Icons.mic_rounded),
          ],
          onChanged: state.updateSource,
        ),
        if (state._source == 'tone') ...[
          const SizedBox(height: 14),
          MobileTextField(
            controller: state._freq,
            label: '测试音频率 (Hz)',
            keyboardType: TextInputType.number,
          ),
        ],
        MobileSwitchRow(
          label: '本地播放收到的音频',
          value: state._output,
          onChanged: state.updateOutput,
        ),
        const SizedBox(height: 10),
        MobilePrimaryButton(
          label: busy ? '启动中…' : '加入组网',
          onPressed: busy ? null : state._start,
          progress: busy,
        ),
      ],
    );
  }
}

class _LiveCard extends StatefulWidget {
  const _LiveCard({required this.model, required this.mesh});

  final AppModel model;
  final MeshState mesh;

  @override
  State<_LiveCard> createState() => _LiveCardState();
}

class _LiveCardState extends State<_LiveCard> {
  double? _pendingTarget;
  double? _pendingMaster;
  Timer? _settle;

  @override
  void dispose() {
    _settle?.cancel();
    super.dispose();
  }

  /// Hold the dragged value locally until the engine reports it back.
  void _commit(double value, VoidCallback apply) {
    apply();
    _settle?.cancel();
    _settle = Timer(const Duration(milliseconds: 800), () {
      if (!mounted) return;
      setState(() {
        _pendingTarget = null;
        _pendingMaster = null;
      });
    });
  }

  @override
  Widget build(BuildContext context) {
    final model = widget.model;
    final mesh = widget.mesh;
    final master = mesh.center.master;
    final targetValue = _pendingTarget ?? mesh.center.targetMs;
    final masterValue = _pendingMaster ?? mesh.center.masterGainDb;
    return MobileCard(
      icon: Icons.lan_rounded,
      title: '组网中',
      subtitle:
          '${modeLabel(mesh.center.info.mode)} · 已运行 ${formatUptime(mesh.seconds)}',
      accent: AppColors.accent,
      children: [
        Row(
          crossAxisAlignment: CrossAxisAlignment.end,
          children: [
            Text(
              formatDb(master.rmsDb),
              style: AppText.mono.copyWith(
                fontSize: 52,
                height: 1,
                fontWeight: FontWeight.w700,
                letterSpacing: -2,
                color: AppColors.text,
              ),
            ),
            const SizedBox(width: 8),
            Padding(
              padding: const EdgeInsets.only(bottom: 8),
              child: Text(
                'dBFS',
                style: AppText.mono.copyWith(
                  color: AppColors.textDim,
                  fontSize: 12,
                ),
              ),
            ),
          ],
        ),
        const SizedBox(height: 12),
        LevelMeter(
          rmsDb: master.rmsDb,
          peakDb: master.peakDb,
          peakHoldDb: master.peakHoldDb,
          height: 14,
          showValue: false,
        ),
        const SizedBox(height: 14),
        Row(
          children: [
            Expanded(
              child: MobileStat(
                label: '节点',
                value: '${mesh.peers.length} 台',
                color: AppColors.accent,
              ),
            ),
            const SizedBox(width: 10),
            Expanded(
              child: MobileStat(
                label: '收到的流',
                value: '${mesh.center.streams.length} 路',
                color: AppColors.accent2,
              ),
            ),
          ],
        ),
        const SizedBox(height: 10),
        Row(
          children: [
            Expanded(
              child: MobileStat(
                label: '上行',
                value: '${mesh.senderKbps.round()} kbps',
              ),
            ),
            const SizedBox(width: 10),
            Expanded(
              child: MobileStat(
                label: '发出',
                value: formatCount(mesh.senderPackets),
              ),
            ),
          ],
        ),
        const SizedBox(height: 18),
        MobileSlider(
          label: '缓冲目标',
          value: targetValue,
          min: 15,
          max: 100,
          display: '${targetValue.round()} ms',
          onChanged: (value) {
            setState(() => _pendingTarget = value);
            _commit(value, () => model.setTargetMs(value));
          },
        ),
        MobileSlider(
          label: '主增益',
          value: masterValue,
          min: -24,
          max: 12,
          display:
              '${masterValue >= 0 ? '+' : ''}${masterValue.toStringAsFixed(1)} dB',
          onChanged: (value) {
            setState(() => _pendingMaster = value);
            _commit(value, () => model.setMasterGainDb(value));
          },
        ),
      ],
    );
  }
}
