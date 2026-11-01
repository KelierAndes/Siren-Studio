import 'package:flutter/material.dart';

import 'controls.dart';
import 'model.dart';
import 'screen_display.dart';
import 'stream_card.dart';
import 'theme.dart';

/// Desktop chassis laid out to the reference proportions: 1180 x 760 canvas,
/// the screen taking ~53 % of the panel, then one row of four cards — two wide
/// device cards plus two narrow system cards — with the HP/LP strip below them.
class PulsarDevice extends StatelessWidget {
  const PulsarDevice({super.key, required this.model, required this.size});

  static const Size design = Size(1180, 760);
  final ShaperModel model;
  final Size size;

  MetalPalette get palette => model.palette;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: model,
      builder: (context, _) {
        return Container(
          width: size.width,
          height: size.height,
          decoration: BoxDecoration(
            gradient: RadialGradient(
              center: const Alignment(-.3, -.6),
              radius: 1.4,
              colors: [palette.deskTop, palette.deskBottom],
            ),
          ),
          child: Stack(
            children: [
              Positioned(
                left: 56,
                top: 26,
                width: 1068,
                height: 40,
                child: _TopBar(model: model, palette: palette),
              ),
              Positioned(
                left: 56,
                top: 78,
                width: 1068,
                height: 628,
                child: _Panel(model: model, palette: palette),
              ),
            ],
          ),
        );
      },
    );
  }
}

/// 顶栏：左品牌，中间状态胶囊（两端是设备翻页箭头），右远端路数。
class _TopBar extends StatelessWidget {
  const _TopBar({required this.model, required this.palette});

  final ShaperModel model;
  final MetalPalette palette;

  @override
  Widget build(BuildContext context) {
    final p = palette;
    final paging = model.streamPages > 1;
    return Row(
      children: [
        SizedBox(
          width: 200,
          child: Text(
            'SIREN MESH',
            style: NeuType.chrome(12, p.ink, tracking: 3.4, w: FontWeight.w700),
          ),
        ),
        Expanded(
          child: Center(
            child: NeuWell(
              size: const Size(420, 32),
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
                      style: NeuType.chrome(12, p.inkStrong, tracking: 1.2, w: FontWeight.w600),
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
            ),
          ),
        ),
        SizedBox(
          width: 200,
          child: Text(
            '${model.onlinePeers} 路远端',
            textAlign: TextAlign.right,
            style: NeuType.chrome(11, p.inkFaint, tracking: .8),
          ),
        ),
      ],
    );
  }
}

class _Panel extends StatelessWidget {
  const _Panel({required this.model, required this.palette});

  final ShaperModel model;
  final MetalPalette palette;

  @override
  Widget build(BuildContext context) {
    return NeuRaised(
      palette: palette,
      radius: 28,
      padding: const EdgeInsets.all(16),
      child: SizedBox(
        width: 1036,
        height: 596,
        child: Column(
          children: [
            ScreenDisplay(
              model: model,
              palette: palette,
              size: const Size(1036, 316),
            ),
            const SizedBox(height: 12),
            SizedBox(
              width: 1036,
              height: 268,
              child: Row(
                children: [
                  _SideFader(
                    label: 'IN',
                    value: model.input,
                    range: ShaperModel.inputRange,
                    onChanged: model.setInput,
                    palette: palette,
                    accent: palette.cyan,
                  ),
                  const SizedBox(width: 10),
                  StreamCard(
                    channel: model.channels[0],
                    palette: palette,
                    size: StreamCard.design,
                  ),
                  const SizedBox(width: 10),
                  StreamCard(
                    channel: model.channels[1],
                    palette: palette,
                    size: StreamCard.design,
                  ),
                  const SizedBox(width: 10),
                  SizedBox(
                    width: 444,
                    height: 268,
                    child: Column(
                      children: [
                        Row(
                          children: [
                            _LocalCard(model: model, palette: palette),
                            const SizedBox(width: 10),
                            _MeshCard(model: model, palette: palette),
                          ],
                        ),
                        const SizedBox(height: 13),
                        _BottomStrip(model: model, palette: palette),
                      ],
                    ),
                  ),
                  const SizedBox(width: 10),
                  _SideFader(
                    label: 'OUT',
                    value: model.output,
                    range: ShaperModel.outputRange,
                    onChanged: model.setOutput,
                    palette: palette,
                    accent: palette.orange,
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// 本机：节点名与音源，不使用旋钮。
class _LocalCard extends StatefulWidget {
  const _LocalCard({required this.model, required this.palette});

  static const Size design = Size(217, 189);
  final ShaperModel model;
  final MetalPalette palette;

  @override
  State<_LocalCard> createState() => _LocalCardState();
}

class _LocalCardState extends State<_LocalCard> {
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
      size: _LocalCard.design,
      radius: 16,
      padding: const EdgeInsets.fromLTRB(14, 12, 14, 12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            height: 22,
            child: Row(
              children: [
                Text('本机', style: NeuType.chrome(16.5, p.inkStrong, tracking: 1.2, w: FontWeight.w700)),
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
              style: NeuType.chrome(13, p.inkStrong, tracking: .2, w: FontWeight.w600),
              decoration: InputDecoration(
                isDense: true,
                filled: true,
                fillColor: p.wellTint,
                contentPadding: const EdgeInsets.symmetric(horizontal: 11, vertical: 8),
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

/// 组网：角色/链路读数与加入按钮，不使用旋钮。
class _MeshCard extends StatelessWidget {
  const _MeshCard({required this.model, required this.palette});

  static const Size design = Size(217, 189);
  final ShaperModel model;
  final MetalPalette palette;

  @override
  Widget build(BuildContext context) {
    final m = model;
    final p = palette;
    return NeuRaised(
      palette: p,
      size: design,
      radius: 16,
      padding: const EdgeInsets.fromLTRB(14, 12, 14, 12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            height: 22,
            child: Row(
              children: [
                Text('组网', style: NeuType.chrome(16.5, p.inkStrong, tracking: 1.2, w: FontWeight.w700)),
                const SizedBox(width: 10),
                NeuLed(palette: p, color: p.violet, on: m.joined, size: 8),
                const Spacer(),
                Text('${m.meshDevices} 台', style: NeuType.numeric(11.5, p.inkFaint)),
              ],
            ),
          ),
          const Spacer(),
          NeuWell(
            size: const Size(189, 48),
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
            width: 189,
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

/// 底栏（参考图的 HP —— LP / DRY / WET / BYPASS）：缓冲、主增益、音调与输出开关。
class _BottomStrip extends StatelessWidget {
  const _BottomStrip({required this.model, required this.palette});

  static const Size design = Size(444, 66);
  final ShaperModel model;
  final MetalPalette palette;

  @override
  Widget build(BuildContext context) {
    final m = model;
    final p = palette;
    return NeuWell(
      size: design,
      radius: 14,
      palette: p,
      tint: p.stripTint,
      padding: const EdgeInsets.fromLTRB(12, 9, 12, 9),
      child: Row(
        children: [
          Expanded(
            child: _stripSlider(
              label: '缓冲',
              value: m.targetMs,
              range: const NeuRange(10, 500),
              unit: 'ms',
              accent: p.cyan,
              onChanged: m.editTarget,
            ),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: _stripSlider(
              label: '主增益',
              value: m.masterGainDb,
              range: const NeuRange(-24, 12),
              unit: 'dB',
              accent: p.orange,
              onChanged: m.editMaster,
            ),
          ),
          const SizedBox(width: 14),
          NeuPill(
            palette: p,
            label: m.outputOn ? '本地播放' : '静默接收',
            height: 30,
            fontSize: 10.5,
            active: m.outputOn,
            accent: p.cyan,
            onTap: m.toggleOutput,
          ),
          const SizedBox(width: 8),
          NeuPill(
            palette: p,
            label: 'BYPASS',
            height: 30,
            fontSize: 10.5,
            led: true,
            active: m.bypass,
            accent: p.salmon,
            onTap: m.toggleBypass,
          ),
        ],
      ),
    );
  }

  Widget _stripSlider({
    required String label,
    required double value,
    required NeuRange range,
    required String unit,
    required Color accent,
    required ValueChanged<double> onChanged,
  }) {
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
                  unit == 'dB' ? '${value.toStringAsFixed(1)} $unit' : '${value.round()} $unit',
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
    return Text(text, style: NeuType.chrome(10, palette.inkFaint, tracking: .6, w: FontWeight.w600));
  }
}

class _SideFader extends StatelessWidget {
  const _SideFader({
    required this.label,
    required this.value,
    required this.range,
    required this.onChanged,
    required this.palette,
    required this.accent,
  });

  final String label;
  final double value;
  final NeuRange range;
  final ValueChanged<double> onChanged;
  final MetalPalette palette;
  final Color accent;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: 34,
      height: 268,
      child: Column(
        children: [
          NeuFader(
            palette: palette,
            value: value,
            range: range,
            accent: accent,
            width: 34,
            height: 212,
            ticks: 2,
            onChanged: onChanged,
          ),
          const SizedBox(height: 8),
          Text(label, style: NeuType.chrome(11, palette.ink, tracking: 2, w: FontWeight.w700)),
          Text(value.toStringAsFixed(1), style: NeuType.numeric(10.5, accent)),
        ],
      ),
    );
  }
}

class IconButton32 extends StatefulWidget {
  const IconButton32({
    super.key,
    required this.palette,
    required this.icon,
    required this.onTap,
    this.size = 32,
  });

  final MetalPalette palette;
  final IconData icon;
  final VoidCallback onTap;
  final double size;

  @override
  State<IconButton32> createState() => _IconButton32State();
}

class _IconButton32State extends State<IconButton32> {
  bool _down = false;

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTapDown: (_) => setState(() => _down = true),
      onTapCancel: () => setState(() => _down = false),
      onTapUp: (_) => setState(() => _down = false),
      onTap: widget.onTap,
      child: SizedBox(
        width: widget.size,
        height: widget.size,
        child: CustomPaint(
          painter: _IconBtnPainter(palette: widget.palette, down: _down),
          child: Icon(
            widget.icon,
            size: widget.size * .58,
            color: _down ? widget.palette.cyan : widget.palette.ink,
          ),
        ),
      ),
    );
  }
}

class _IconBtnPainter extends CustomPainter {
  _IconBtnPainter({required this.palette, required this.down});

  final MetalPalette palette;
  final bool down;

  @override
  void paint(Canvas canvas, Size size) {
    final r = RRect.fromRectAndRadius(Offset.zero & size, const Radius.circular(9));
    if (down) {
      canvas.drawRRect(r, Paint()..color = palette.stripTint);
      canvas.save();
      canvas.clipRRect(r);
      canvas.drawRRect(
        r.shift(const Offset(0, 1.6)),
        Paint()
          ..style = PaintingStyle.stroke
          ..strokeWidth = 6
          ..color = palette.shade(.8)
          ..maskFilter = const MaskFilter.blur(BlurStyle.normal, 3),
      );
      canvas.restore();
    } else {
      canvas.drawRRect(
        r.shift(const Offset(0, 3)),
        Paint()
          ..color = palette.shade(.6)
          ..maskFilter = const MaskFilter.blur(BlurStyle.normal, 5),
      );
      canvas.drawRRect(
        r,
        Paint()
          ..shader = LinearGradient(
            begin: Alignment.topCenter,
            end: Alignment.bottomCenter,
            colors: [palette.moduleTop, palette.moduleBottom],
          ).createShader(Offset.zero & size),
      );
      canvas.drawRRect(
        r,
        Paint()
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1
          ..color = palette.bevel.withValues(alpha: .5),
      );
    }
  }

  @override
  bool shouldRepaint(_IconBtnPainter old) => old.down != down || old.palette != palette;
}
