import 'package:flutter/material.dart';

import 'controls.dart';
import 'model.dart';
import 'screen_display.dart';
import 'stream_card.dart';
import 'theme.dart';

/// Desktop chassis: 1180 x 760 design canvas.
///
/// Two paged stream cards on the left, EQ screen above, and the join/audio
/// controls laid out inline in the right-hand 本机 / 组网 cards.
class PulsarDevice extends StatefulWidget {
  const PulsarDevice({super.key, required this.model, required this.size});

  static const Size design = Size(1180, 760);
  final ShaperModel model;
  final Size size;

  @override
  State<PulsarDevice> createState() => _PulsarDeviceState();
}

class _PulsarDeviceState extends State<PulsarDevice> with SingleTickerProviderStateMixin {
  late final AnimationController _clock = AnimationController(
    vsync: this,
    duration: const Duration(seconds: 12),
  )..repeat();

  MetalPalette get palette => widget.model.palette;

  @override
  void dispose() {
    _clock.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final m = widget.model;
    return ListenableBuilder(
      listenable: m,
      builder: (context, _) {
        return Container(
          width: widget.size.width,
          height: widget.size.height,
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
                top: 24,
                width: 1068,
                height: 40,
                child: _TopBar(model: m, palette: palette),
              ),
              Positioned(
                left: 56,
                top: 76,
                width: 1068,
                height: 628,
                child: _Panel(model: m, palette: palette, clock: _clock),
              ),
            ],
          ),
        );
      },
    );
  }
}

class _TopBar extends StatelessWidget {
  const _TopBar({required this.model, required this.palette});

  final ShaperModel model;
  final MetalPalette palette;

  @override
  Widget build(BuildContext context) {
    return Row(
      children: [
        Text(
          'SIREN MESH',
          style: NeuType.chrome(12, palette.ink, tracking: 3.4, w: FontWeight.w700),
        ),
        if (model.streamPages > 1) ...[
          const SizedBox(width: 16),
          _PageWell(model: model, palette: palette),
        ],
        const Spacer(),
        NeuWell(
          size: const Size(300, 32),
          radius: 16,
          palette: palette,
          tint: palette.wellTint,
          padding: const EdgeInsets.symmetric(horizontal: 6),
          child: Row(
            children: [
              TinyRoundButton(
                palette: palette,
                on: model.joined,
                color: palette.ledOn,
                size: 18,
                onTap: model.toggleJoin,
              ),
              const SizedBox(width: 8),
              Expanded(
                child: Text(
                  model.statusText,
                  textAlign: TextAlign.center,
                  style: NeuType.chrome(12, palette.inkStrong, tracking: 1.2, w: FontWeight.w600),
                  maxLines: 1,
                  overflow: TextOverflow.clip,
                ),
              ),
            ],
          ),
        ),
        const Spacer(),
        Text(
          '${model.onlinePeers} 路远端',
          style: NeuType.chrome(10.5, palette.inkFaint, tracking: .8),
        ),
      ],
    );
  }
}

class _PageWell extends StatelessWidget {
  const _PageWell({required this.model, required this.palette});

  final ShaperModel model;
  final MetalPalette palette;

  @override
  Widget build(BuildContext context) {
    return NeuWell(
      size: const Size(104, 26),
      radius: 13,
      palette: palette,
      tint: palette.wellTint,
      padding: const EdgeInsets.symmetric(horizontal: 2, vertical: 2),
      child: Row(
        children: [
          IconButton32(
            palette: palette,
            icon: Icons.chevron_left_rounded,
            size: 22,
            onTap: () => model.setStreamPage(model.streamPage - 1),
          ),
          Expanded(
            child: Text(
              '${model.streamPage + 1}/${model.streamPages}',
              textAlign: TextAlign.center,
              style: NeuType.numeric(10.5, palette.ink),
            ),
          ),
          IconButton32(
            palette: palette,
            icon: Icons.chevron_right_rounded,
            size: 22,
            onTap: () => model.setStreamPage(model.streamPage + 1),
          ),
        ],
      ),
    );
  }
}

class _Panel extends StatelessWidget {
  const _Panel({required this.model, required this.palette, required this.clock});

  final ShaperModel model;
  final MetalPalette palette;
  final Animation<double> clock;

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
              size: const Size(1036, 244),
            ),
            const SizedBox(height: 12),
            SizedBox(
              width: 1036,
              height: 340,
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
                    size: const Size(210, 340),
                  ),
                  const SizedBox(width: 8),
                  StreamCard(
                    channel: model.channels[1],
                    palette: palette,
                    size: const Size(210, 340),
                  ),
                  const SizedBox(width: 12),
                  _LocalLargeCard(model: model, palette: palette),
                  const SizedBox(width: 12),
                  _MeshLargeCard(model: model, palette: palette),
                  const SizedBox(width: 8),
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

class _LocalLargeCard extends StatefulWidget {
  const _LocalLargeCard({required this.model, required this.palette});

  final ShaperModel model;
  final MetalPalette palette;

  @override
  State<_LocalLargeCard> createState() => _LocalLargeCardState();
}

class _LocalLargeCardState extends State<_LocalLargeCard> {
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
      size: const Size(237, 340),
      radius: 14,
      padding: const EdgeInsets.fromLTRB(16, 14, 16, 14),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            height: 26,
            child: Row(
              children: [
                Text('本机', style: NeuType.chrome(17.5, p.inkStrong, tracking: 1.4, w: FontWeight.w700)),
                const SizedBox(width: 10),
                NeuLed(palette: p, color: p.ledOn, on: m.joined, size: 8),
                const SizedBox(width: 7),
                Expanded(
                  child: Text(
                    m.statusText,
                    overflow: TextOverflow.ellipsis,
                    style: NeuType.chrome(11, p.inkFaint, tracking: .6),
                  ),
                ),
              ],
            ),
          ),
          const SizedBox(height: 14),
          _SectionLabel('节点名称', palette: p),
          const SizedBox(height: 6),
          SizedBox(
            height: 36,
            child: TextField(
              controller: _name,
              focusNode: _focus,
              enabled: !m.joined,
              onChanged: m.editNodeName,
              style: NeuType.chrome(13.5, p.inkStrong, tracking: .2, w: FontWeight.w600),
              decoration: InputDecoration(
                isDense: true,
                filled: true,
                fillColor: p.wellTint,
                contentPadding: const EdgeInsets.symmetric(horizontal: 12, vertical: 9),
                hintText: 'Siren Node',
                hintStyle: NeuType.chrome(12.5, p.inkFaint),
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
          const SizedBox(height: 14),
          _SectionLabel('音源', palette: p),
          const SizedBox(height: 8),
          FittedBox(
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
                    height: 30,
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
          const SizedBox(height: 14),
          _SectionLabel('音调', palette: p),
          const SizedBox(height: 6),
          SizedBox(
            height: 26,
            child: Row(
              children: [
                Expanded(
                  child: LayoutBuilder(
                    builder: (context, box) => NeuSlider(
                      palette: p,
                      value: m.toneFreq,
                      range: const NeuRange(20, 1000),
                      width: box.maxWidth,
                      height: 26,
                      accentColor: p.orange,
                      onChanged: m.joined ? (_) {} : m.editToneFreq,
                    ),
                  ),
                ),
                const SizedBox(width: 8),
                SizedBox(
                  width: 58,
                  child: Text(
                    '${m.toneFreq.round()} Hz',
                    textAlign: TextAlign.right,
                    style: NeuType.numeric(11.5, p.cyan),
                  ),
                ),
              ],
            ),
          ),
          const Spacer(),
          SizedBox(
            width: 205,
            height: 34,
            child: NeuPill(
              palette: p,
              label: m.outputOn ? '本地播放' : '静默接收',
              height: 34,
              fontSize: 12.5,
              fill: true,
              active: m.outputOn,
              accent: p.cyan,
              onTap: m.toggleOutput,
            ),
          ),
        ],
      ),
    );
  }
}

class _MeshLargeCard extends StatelessWidget {
  const _MeshLargeCard({required this.model, required this.palette});

  final ShaperModel model;
  final MetalPalette palette;

  @override
  Widget build(BuildContext context) {
    final m = model;
    final p = palette;
    return NeuRaised(
      palette: p,
      size: const Size(237, 340),
      radius: 14,
      padding: const EdgeInsets.fromLTRB(16, 14, 16, 14),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            height: 26,
            child: Row(
              children: [
                Text('组网', style: NeuType.chrome(17.5, p.inkStrong, tracking: 1.4, w: FontWeight.w700)),
                const SizedBox(width: 10),
                NeuLed(palette: p, color: p.violet, on: m.joined, size: 8),
                const SizedBox(width: 7),
                Expanded(
                  child: Text(
                    m.meshDetail.isEmpty ? '未连接' : m.meshDetail,
                    overflow: TextOverflow.ellipsis,
                    style: NeuType.chrome(11, p.inkFaint, tracking: .6),
                  ),
                ),
                Text(
                  '${m.meshDevices} 台',
                  style: NeuType.numeric(11, p.inkFaint),
                ),
              ],
            ),
          ),
          const SizedBox(height: 14),
          _SettingSlider(
            palette: p,
            label: '缓冲',
            value: m.targetMs,
            range: const NeuRange(10, 500),
            unit: 'ms',
            accent: p.cyan,
            onChanged: m.editTarget,
          ),
          const SizedBox(height: 12),
          _SettingSlider(
            palette: p,
            label: '主增益',
            value: m.masterGainDb,
            range: const NeuRange(-24, 12),
            unit: 'dB',
            accent: p.orange,
            onChanged: m.editMaster,
          ),
          const Spacer(),
          SizedBox(
            width: 205,
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
          const SizedBox(height: 8),
          Text(
            m.joined ? '切换设置后重新加入生效' : '选择音源后加入局域网组网',
            maxLines: 2,
            overflow: TextOverflow.ellipsis,
            style: NeuType.chrome(10.5, p.inkFaint, tracking: .3),
          ),
        ],
      ),
    );
  }
}

class _SectionLabel extends StatelessWidget {
  const _SectionLabel(this.text, {required this.palette});

  final String text;
  final MetalPalette palette;

  @override
  Widget build(BuildContext context) {
    return Text(text, style: NeuType.chrome(10.5, palette.inkFaint, tracking: .6, w: FontWeight.w600));
  }
}

class _SettingSlider extends StatelessWidget {
  const _SettingSlider({
    required this.palette,
    required this.label,
    required this.value,
    required this.range,
    required this.unit,
    required this.accent,
    required this.onChanged,
  });

  final MetalPalette palette;
  final String label;
  final double value;
  final NeuRange range;
  final String unit;
  final Color accent;
  final ValueChanged<double> onChanged;

  @override
  Widget build(BuildContext context) {
    return Row(
      children: [
        SizedBox(
          width: 40,
          child: Text(label, style: NeuType.chrome(10.5, palette.inkFaint, tracking: .4)),
        ),
        const SizedBox(width: 8),
        Expanded(
          child: LayoutBuilder(
            builder: (context, box) => NeuSlider(
              palette: palette,
              value: value,
              range: range,
              width: box.maxWidth,
              height: 26,
              accentColor: accent,
              onChanged: onChanged,
            ),
          ),
        ),
        const SizedBox(width: 8),
        SizedBox(
          width: 58,
          child: Text(
            unit == 'dB' ? '${value.toStringAsFixed(1)} dB' : '${value.round()} ms',
            textAlign: TextAlign.right,
            style: NeuType.numeric(11.5, accent),
          ),
        ),
      ],
    );
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
      width: 46,
      height: 340,
      child: Column(
        children: [
          NeuFader(
            palette: palette,
            value: value,
            range: range,
            accent: accent,
            width: 46,
            height: 282,
            onChanged: onChanged,
          ),
          const SizedBox(height: 8),
          Text(label, style: NeuType.chrome(11.5, palette.ink, tracking: 2, w: FontWeight.w700)),
          Text(
            value.toStringAsFixed(1),
            style: NeuType.numeric(11, accent),
          ),
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
