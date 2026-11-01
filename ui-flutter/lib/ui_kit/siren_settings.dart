import 'package:flutter/material.dart';

import '../app_model.dart';
import '../models.dart';
import 'controls.dart';
import 'model.dart';
import 'siren_binding.dart';
import 'theme.dart';

Future<void> showSirenSettings(
  BuildContext context,
  SirenBinding binding,
) {
  return showDialog<void>(
    context: context,
    barrierColor: Colors.black.withValues(alpha: .45),
    builder: (context) => Dialog(
      backgroundColor: Colors.transparent,
      insetPadding: const EdgeInsets.all(18),
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 540, maxHeight: 720),
        child: SirenSettingsSheet(binding: binding),
      ),
    ),
  );
}

class SirenSettingsSheet extends StatefulWidget {
  const SirenSettingsSheet({super.key, required this.binding});

  final SirenBinding binding;

  @override
  State<SirenSettingsSheet> createState() => _SirenSettingsSheetState();
}

class _SirenSettingsSheetState extends State<SirenSettingsSheet> {
  late final TextEditingController _name;
  late String _source;
  late bool _output;
  late double _freq;
  late double _target;
  late double _master;

  AppModel get app => widget.binding.app;
  ShaperModel get visual => widget.binding.visual;

  @override
  void initState() {
    super.initState();
    _name = TextEditingController(text: app.nodeName);
    _source = app.lastSource;
    _output = app.lastOutput;
    _freq = app.lastFreq;
    _target = app.targetMs;
    _master = app.masterGainDb;
  }

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  Future<void> _join() async {
    final name = _name.text.trim().isEmpty ? 'Siren Node' : _name.text.trim();
    await app.startMesh(
      name: name,
      source: _source,
      output: _output,
      freq: _freq,
    );
    if (!mounted) return;
    setState(() {
      _target = app.targetMs;
      _master = app.masterGainDb;
    });
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: Listenable.merge([app, visual]),
      builder: (context, _) {
        final p = visual.palette;
        final running = app.meshRunning;
        final peers = app.mesh?.peers ?? const [];
        return NeuRaised(
          palette: p,
          radius: 20,
          padding: EdgeInsets.zero,
          child: ClipRRect(
            borderRadius: BorderRadius.circular(20),
            child: Container(
              color: p.isLight ? p.moduleTop : p.moduleBottom,
              child: SingleChildScrollView(
                padding: const EdgeInsets.fromLTRB(22, 18, 22, 20),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    _header(p, running),
                    const SizedBox(height: 16),
                    _label(p, '节点名称'),
                    _nameField(p),
                    const SizedBox(height: 14),
                    _label(p, '本机音频来源'),
                    _sourceRow(p, running),
                    const SizedBox(height: 14),
                    _slider(
                      p,
                      '测试音频率',
                      _freq,
                      const NeuRange(20, 1000),
                      (v) {
                        setState(() => _freq = v);
                        app.lastFreq = v;
                      },
                      '${_freq.round()} Hz',
                      enabled: !running,
                    ),
                    _slider(
                      p,
                      '缓冲目标',
                      _target,
                      const NeuRange(10, 500),
                      (v) {
                        setState(() => _target = v);
                        app.setTargetMs(v);
                      },
                      '${_target.round()} ms',
                    ),
                    _slider(
                      p,
                      '主增益',
                      _master,
                      const NeuRange(-24, 12),
                      (v) {
                        setState(() => _master = v);
                        app.setMasterGainDb(v);
                      },
                      '${_master.toStringAsFixed(1)} dB',
                    ),
                    const SizedBox(height: 6),
                    Row(
                      children: [
                        _label(p, '本地播放收到的音频', inline: true),
                        const Spacer(),
                        NeuPill(
                          palette: p,
                          label: _output ? '开启' : '关闭',
                          height: 28,
                          fontSize: 11.5,
                          led: true,
                          active: _output,
                          accent: p.cyan,
                          onTap: running
                              ? () {}
                              : () => setState(() => _output = !_output),
                        ),
                      ],
                    ),
                    const SizedBox(height: 12),
                    Row(
                      children: [
                        _label(p, '主题', inline: true),
                        const Spacer(),
                        NeuWell(
                          size: const Size(120, 28),
                          radius: 14,
                          palette: p,
                          tint: p.wellTint,
                          padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 3),
                          child: Row(
                            children: [
                              for (final e in const [
                                (AppThemeMode.dark, '暗色'),
                                (AppThemeMode.light, '亮色'),
                              ])
                                NeuPill(
                                  palette: p,
                                  label: e.$2,
                                  height: 22,
                                  fontSize: 11,
                                  active: visual.theme == e.$1,
                                  onTap: () => visual.setTheme(e.$1),
                                ),
                            ],
                          ),
                        ),
                      ],
                    ),
                    const SizedBox(height: 16),
                    _peers(p, peers),
                    if (app.lastError != null) ...[
                      const SizedBox(height: 10),
                      Text(
                        app.lastError!,
                        style: NeuType.chrome(11.5, p.salmon, tracking: .3),
                      ),
                    ],
                    const SizedBox(height: 18),
                    Row(
                      children: [
                        Expanded(
                          child: NeuPill(
                            palette: p,
                            label: app.busyMesh
                                ? '处理中…'
                                : (running ? '离开组网' : '加入组网'),
                            height: 40,
                            fontSize: 13,
                            fill: true,
                            accent: running ? p.salmon : p.cyan,
                            active: true,
                            onTap: app.busyMesh
                                ? () {}
                                : (running ? app.stopMesh : _join),
                          ),
                        ),
                        const SizedBox(width: 10),
                        NeuPill(
                          palette: p,
                          label: '关闭',
                          height: 40,
                          fontSize: 13,
                          onTap: () => Navigator.of(context).pop(),
                        ),
                      ],
                    ),
                    if (running) ...[
                      const SizedBox(height: 8),
                      Text(
                        '修改来源或频率后，离开并重新加入才会生效。',
                        style: NeuType.chrome(10.5, p.inkFaint, tracking: .2),
                      ),
                    ],
                  ],
                ),
              ),
            ),
          ),
        );
      },
    );
  }

  Widget _header(MetalPalette p, bool running) {
    return Row(
      children: [
        NeuLed(palette: p, color: p.cyan, on: running, size: 9),
        const SizedBox(width: 9),
        Text(
          'SIREN STUDIO',
          style: NeuType.chrome(14, p.inkStrong, tracking: 2.6, w: FontWeight.w800),
        ),
        const SizedBox(width: 10),
        Expanded(
          child: Text(
            visual.statusText,
            overflow: TextOverflow.ellipsis,
            style: NeuType.chrome(11, p.inkFaint, tracking: .6),
          ),
        ),
        GestureDetector(
          onTap: () => Navigator.of(context).pop(),
          child: Icon(Icons.close_rounded, size: 20, color: p.inkFaint),
        ),
      ],
    );
  }

  Widget _label(MetalPalette p, String text, {bool inline = false}) {
    return Padding(
      padding: EdgeInsets.only(bottom: inline ? 0 : 7),
      child: Text(
        text,
        style: NeuType.chrome(11, p.inkFaint, tracking: 1.2, w: FontWeight.w600),
      ),
    );
  }

  Widget _nameField(MetalPalette p) {
    return TextField(
      controller: _name,
      enabled: !app.meshRunning,
      style: NeuType.chrome(13.5, p.inkStrong, tracking: .2, w: FontWeight.w600),
      decoration: InputDecoration(
        isDense: true,
        filled: true,
        fillColor: p.wellTint,
        contentPadding: const EdgeInsets.symmetric(horizontal: 12, vertical: 12),
        hintText: 'Siren Node',
        hintStyle: NeuType.chrome(12.5, p.inkFaint),
        border: OutlineInputBorder(
          borderRadius: BorderRadius.circular(10),
          borderSide: BorderSide(color: p.bevel),
        ),
        enabledBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(10),
          borderSide: BorderSide(color: p.bevel),
        ),
        focusedBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(10),
          borderSide: BorderSide(color: p.cyan, width: 1.2),
        ),
      ),
    );
  }

  Widget _sourceRow(MetalPalette p, bool running) {
    const options = [
      ('tone', '测试音'),
      ('loopback', '系统内录'),
      ('microphone', '麦克风'),
    ];
    return Row(
      children: [
        for (final option in options) ...[
          Expanded(
            child: NeuPill(
              palette: p,
              label: option.$2,
              height: 34,
              fontSize: 11.5,
              fill: true,
              active: _source == option.$1,
              accent: p.cyan,
              onTap: running ? () {} : () => setState(() => _source = option.$1),
            ),
          ),
          if (option != options.last) const SizedBox(width: 8),
        ],
      ],
    );
  }

  Widget _slider(
    MetalPalette p,
    String label,
    double value,
    NeuRange range,
    ValueChanged<double> onChanged,
    String readout, {
    bool enabled = true,
  }) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Text(
                label,
                style: NeuType.chrome(11, p.inkFaint, tracking: 1.2, w: FontWeight.w600),
              ),
              const Spacer(),
              Text(
                readout,
                style: NeuType.numeric(11.5, enabled ? p.cyan : p.inkFaint),
              ),
            ],
          ),
          const SizedBox(height: 6),
          LayoutBuilder(
            builder: (context, box) => NeuSlider(
              palette: p,
              value: value,
              range: range,
              width: box.maxWidth,
              height: 24,
              accentColor: enabled ? p.cyan : p.inkFaint,
              onChanged: enabled ? onChanged : (_) {},
            ),
          ),
        ],
      ),
    );
  }

  Widget _peers(MetalPalette p, List<MeshPeerState> peers) {
    if (peers.isEmpty) {
      return Text(
        '正在扫描局域网…',
        style: NeuType.chrome(11.5, p.inkFaint),
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        _label(p, '已连接节点'),
        for (final peer in peers)
          Padding(
            padding: const EdgeInsets.only(top: 6),
            child: Row(
              children: [
                NeuLed(palette: p, color: p.cyan, on: true, size: 6),
                const SizedBox(width: 8),
                Expanded(
                  child: Text(
                    peer.displayName,
                    style: NeuType.chrome(12, p.ink, tracking: .2, w: FontWeight.w600),
                  ),
                ),
                Text(
                  peer.bt == null ? 'Wi-Fi' : '双栈',
                  style: NeuType.chrome(10, p.inkFaint, tracking: .8),
                ),
              ],
            ),
          ),
      ],
    );
  }
}
