import 'package:flutter/material.dart';

import 'controls.dart';
import 'model.dart';
import 'theme.dart';

/// 三端共用的面板组件：顶栏状态胶囊、本机卡、组网卡与底栏。
///
/// 桌面（`device.dart`）、手机（`mobile_device.dart`）与 Linux WebUI 用的是同
/// 一套结构与文案，只有尺寸不同；WebUI 侧对应 `webui/index.html` 里的
/// `.pill-bar` / `.card--local` / `.card--mesh` / `.strip`。

/// 顶栏中央的状态胶囊：`组网中 · N 台 · 页码`，两端是设备翻页箭头。
class TopStatusPill extends StatelessWidget {
  const TopStatusPill({
    super.key,
    required this.model,
    required this.palette,
    this.width = 420,
  });

  final ShaperModel model;
  final MetalPalette palette;
  final double width;

  @override
  Widget build(BuildContext context) {
    final p = palette;
    final paging = model.streamPages > 1;
    return NeuWell(
      size: Size(width, 32),
      radius: 16,
      palette: p,
      tint: p.wellTint,
      padding: const EdgeInsets.symmetric(horizontal: 4),
      child: Row(
        children: [
          SizedBox(
            width: 24,
            child: paging
                ? IconButton32(
                    palette: p,
                    icon: Icons.chevron_left_rounded,
                    size: 22,
                    onTap: () => model.setStreamPage(model.streamPage - 1),
                  )
                : null,
          ),
          Expanded(
            child: Text(
              paging
                  ? '${model.statusText}　·　${model.streamPage + 1}/${model.streamPages}'
                  : model.statusText,
              textAlign: TextAlign.center,
              maxLines: 1,
              overflow: TextOverflow.clip,
              style: NeuType.chrome(
                12,
                p.inkStrong,
                tracking: 1.2,
                w: FontWeight.w600,
              ),
            ),
          ),
          SizedBox(
            width: 24,
            child: paging
                ? IconButton32(
                    palette: p,
                    icon: Icons.chevron_right_rounded,
                    size: 22,
                    onTap: () => model.setStreamPage(model.streamPage + 1),
                  )
                : null,
          ),
        ],
      ),
    );
  }
}

/// 本机卡：节点名、音源、音调，不使用旋钮。
class LocalCard extends StatefulWidget {
  const LocalCard({
    super.key,
    required this.model,
    required this.palette,
    this.size = const Size(217, 189),
  });

  final ShaperModel model;
  final MetalPalette palette;
  final Size size;

  @override
  State<LocalCard> createState() => _LocalCardState();
}

class _LocalCardState extends State<LocalCard> {
  late final TextEditingController _name;
  late final FocusNode _focus;

  @override
  void initState() {
    super.initState();
    _name = TextEditingController(text: widget.model.nodeName);
    _focus = FocusNode();
    widget.model.addListener(_syncName);
    // Windows 激活窗口时会把焦点送给第一个可聚焦控件；名字输入框不该抢焦点。
    WidgetsBinding.instance.addPostFrameCallback((_) => _focus.unfocus());
  }

  void _syncName() {
    if (!_focus.hasFocus && _name.text != widget.model.nodeName) {
      _name.text = widget.model.nodeName;
    }
  }

  @override
  void dispose() {
    widget.model.removeListener(_syncName);
    _name.dispose();
    _focus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final m = widget.model;
    final p = widget.palette;
    return NeuRaised(
      palette: p,
      size: widget.size,
      radius: 16,
      padding: const EdgeInsets.fromLTRB(14, 12, 14, 12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            height: 22,
            child: Row(
              children: [
                Text(
                  '本机',
                  style: NeuType.chrome(
                    16.5,
                    p.inkStrong,
                    tracking: 1.2,
                    w: FontWeight.w700,
                  ),
                ),
                const SizedBox(width: 10),
                NeuLed(palette: p, color: p.ledOn, on: m.joined, size: 8),
                const Spacer(),
                Text(
                  m.busy ? '处理中…' : (m.joined ? '已入网' : '未入网'),
                  style: NeuType.chrome(10.5, p.inkFaint, tracking: .4),
                ),
              ],
            ),
          ),
          const Spacer(),
          _SectionLabel('节点名称', palette: p),
          const SizedBox(height: 5),
          SizedBox(
            height: 32,
            child: TextField(
              controller: _name,
              focusNode: _focus,
              enabled: !m.joined,
              onChanged: m.editNodeName,
              style: NeuType.chrome(
                13,
                p.inkStrong,
                tracking: .2,
                w: FontWeight.w600,
              ),
              decoration: InputDecoration(
                isDense: true,
                filled: true,
                fillColor: p.wellTint,
                contentPadding: const EdgeInsets.symmetric(
                  horizontal: 11,
                  vertical: 8,
                ),
                hintText: 'Siren Node',
                hintStyle: NeuType.chrome(12, p.inkFaint),
                border: OutlineInputBorder(
                  borderRadius: BorderRadius.circular(8),
                  borderSide: BorderSide(color: p.bevel),
                ),
                enabledBorder: OutlineInputBorder(
                  borderRadius: BorderRadius.circular(8),
                  borderSide: BorderSide(color: p.bevel),
                ),
                focusedBorder: OutlineInputBorder(
                  borderRadius: BorderRadius.circular(8),
                  borderSide: BorderSide(color: p.cyan, width: 1.1),
                ),
              ),
            ),
          ),
          const Spacer(),
          _SectionLabel('音源', palette: p),
          const SizedBox(height: 4),
          SizedBox(
            height: 28,
            child: FittedBox(
              fit: BoxFit.scaleDown,
              alignment: Alignment.centerLeft,
              child: Row(
                children: [
                  for (final option in const [
                    ('tone', '测试音'),
                    ('loopback', '系统内录'),
                    ('microphone', '麦克风'),
                  ]) ...[
                    NeuPill(
                      palette: p,
                      label: option.$2,
                      height: 28,
                      fontSize: 10.5,
                      active: m.sourceKey == option.$1,
                      accent: p.cyan,
                      onTap: m.joined ? () {} : () => m.selectSource(option.$1),
                    ),
                    const SizedBox(width: 6),
                  ],
                ],
              ),
            ),
          ),
          const SizedBox(height: 8),
          _SectionLabel('音调', palette: p),
          const SizedBox(height: 3),
          SizedBox(
            height: 22,
            child: Row(
              children: [
                Expanded(
                  child: LayoutBuilder(
                    builder: (context, box) => NeuSlider(
                      palette: p,
                      value: m.toneFreq,
                      range: const NeuRange(20, 1000),
                      width: box.maxWidth,
                      height: 22,
                      accentColor: p.violet,
                      onChanged: m.joined ? (_) {} : m.editToneFreq,
                    ),
                  ),
                ),
                const SizedBox(width: 6),
                SizedBox(
                  width: 52,
                  child: FittedBox(
                    fit: BoxFit.scaleDown,
                    alignment: Alignment.centerRight,
                    child: Text(
                      '${m.toneFreq.round()} Hz',
                      textAlign: TextAlign.right,
                      style: NeuType.numeric(11, p.cyan),
                    ),
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// 组网卡：角色/链路读数与加入按钮，不使用旋钮。
class MeshCard extends StatelessWidget {
  const MeshCard({
    super.key,
    required this.model,
    required this.palette,
    this.size = const Size(217, 189),
  });

  final ShaperModel model;
  final MetalPalette palette;
  final Size size;

  @override
  Widget build(BuildContext context) {
    final m = model;
    final p = palette;
    final innerWidth = size.width - 28;
    return NeuRaised(
      palette: p,
      size: size,
      radius: 16,
      padding: const EdgeInsets.fromLTRB(14, 12, 14, 12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            height: 22,
            child: Row(
              children: [
                Text(
                  '组网',
                  style: NeuType.chrome(
                    16.5,
                    p.inkStrong,
                    tracking: 1.2,
                    w: FontWeight.w700,
                  ),
                ),
                const SizedBox(width: 10),
                NeuLed(palette: p, color: p.violet, on: m.joined, size: 8),
                const Spacer(),
                Text(
                  '${m.meshDevices} 台',
                  style: NeuType.numeric(11.5, p.inkFaint),
                ),
              ],
            ),
          ),
          const Spacer(),
          NeuWell(
            size: Size(innerWidth, 48),
            radius: 10,
            palette: p,
            tint: p.wellTint,
            padding: const EdgeInsets.symmetric(horizontal: 9, vertical: 7),
            child: Column(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                _readout('角色', m.meshRole),
                _readout('链路', m.meshLink),
              ],
            ),
          ),
          const Spacer(),
          SizedBox(
            width: innerWidth,
            height: 34,
            child: NeuPill(
              palette: p,
              label: m.joinLabel,
              height: 34,
              fontSize: 13,
              fill: true,
              active: true,
              accent: m.joined ? p.salmon : p.cyan,
              onTap: m.toggleJoin,
            ),
          ),
        ],
      ),
    );
  }

  Widget _readout(String label, String value) {
    return Row(
      children: [
        Text(label, style: NeuType.chrome(9.5, palette.inkFaint, tracking: .4)),
        Expanded(
          child: Text(
            value,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            textAlign: TextAlign.right,
            style: NeuType.numeric(11.5, palette.ink),
          ),
        ),
      ],
    );
  }
}

/// 底栏（参考图 HP —— LP / DRY / WET / BYPASS）：缓冲、主增益与输出开关。
class BottomStrip extends StatelessWidget {
  const BottomStrip({
    super.key,
    required this.model,
    required this.palette,
    this.size = const Size(444, 66),
    this.twoRow = false,
  });

  final ShaperModel model;
  final MetalPalette palette;
  final Size size;

  /// 手机宽度放不下“两条滑块 + 两枚胶囊”一行时，改成上下两行。
  final bool twoRow;

  @override
  Widget build(BuildContext context) {
    final m = model;
    final p = palette;
    return NeuWell(
      size: size,
      radius: 14,
      palette: p,
      tint: p.stripTint,
      padding: const EdgeInsets.fromLTRB(12, 9, 12, 9),
      child: twoRow
          ? Column(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Row(
                  children: [
                    Expanded(
                      child: _slider(
                        '缓冲',
                        m.targetMs,
                        const NeuRange(10, 500),
                        'ms',
                        p.cyan,
                        m.editTarget,
                      ),
                    ),
                    const SizedBox(width: 14),
                    Expanded(
                      child: _slider(
                        '主增益',
                        m.masterGainDb,
                        const NeuRange(-24, 12),
                        'dB',
                        p.orange,
                        m.editMaster,
                      ),
                    ),
                  ],
                ),
                Row(
                  children: [
                    Expanded(child: _outputPill(m, p)),
                    const SizedBox(width: 8),
                    Expanded(child: _bypassPill(m, p)),
                  ],
                ),
              ],
            )
          : Row(
              children: [
                Expanded(
                  child: _slider(
                    '缓冲',
                    m.targetMs,
                    const NeuRange(10, 500),
                    'ms',
                    p.cyan,
                    m.editTarget,
                  ),
                ),
                const SizedBox(width: 12),
                Expanded(
                  child: _slider(
                    '主增益',
                    m.masterGainDb,
                    const NeuRange(-24, 12),
                    'dB',
                    p.orange,
                    m.editMaster,
                  ),
                ),
                const SizedBox(width: 14),
                _outputPill(m, p),
                const SizedBox(width: 8),
                _bypassPill(m, p),
              ],
            ),
    );
  }

  Widget _outputPill(ShaperModel m, MetalPalette p) => NeuPill(
    palette: p,
    label: m.outputOn ? '本地播放' : '静默接收',
    height: 30,
    fontSize: 10.5,
    active: m.outputOn,
    accent: p.cyan,
    onTap: m.toggleOutput,
  );

  Widget _bypassPill(ShaperModel m, MetalPalette p) => NeuPill(
    palette: p,
    label: 'BYPASS',
    height: 30,
    fontSize: 10.5,
    led: true,
    active: m.bypass,
    accent: p.salmon,
    onTap: m.toggleBypass,
  );

  Widget _slider(
    String label,
    double value,
    NeuRange range,
    String unit,
    Color accent,
    ValueChanged<double> onChanged,
  ) {
    final p = palette;
    return Column(
      mainAxisAlignment: MainAxisAlignment.center,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Row(
          children: [
            Text(label, style: NeuType.chrome(9.5, p.inkFaint, tracking: .4)),
            const Spacer(),
            SizedBox(
              width: 58,
              child: FittedBox(
                fit: BoxFit.scaleDown,
                alignment: Alignment.centerRight,
                child: Text(
                  unit == 'dB'
                      ? '${value.toStringAsFixed(1)} $unit'
                      : '${value.round()} $unit',
                  style: NeuType.numeric(11.5, accent),
                ),
              ),
            ),
          ],
        ),
        const SizedBox(height: 5),
        LayoutBuilder(
          builder: (context, box) => NeuSlider(
            palette: p,
            value: value,
            range: range,
            width: box.maxWidth,
            height: 22,
            accentColor: accent,
            onChanged: onChanged,
          ),
        ),
      ],
    );
  }
}

class _SectionLabel extends StatelessWidget {
  const _SectionLabel(this.text, {required this.palette});

  final String text;
  final MetalPalette palette;

  @override
  Widget build(BuildContext context) {
    return Text(
      text,
      style: NeuType.chrome(
        10,
        palette.inkFaint,
        tracking: .6,
        w: FontWeight.w600,
      ),
    );
  }
}
