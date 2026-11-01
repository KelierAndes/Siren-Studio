import 'dart:math' as math;

import 'package:flutter/material.dart';

import 'controls.dart';
import 'model.dart';
import 'theme.dart';

/// 下半区的一个处理模块（SUB 1 / SUB 2 / TONE / STABILISE）
class ModuleBlock extends StatelessWidget {
  const ModuleBlock({
    super.key,
    required this.channel,
    required this.model,
    required this.palette,
    required this.size,
    required this.clock,
  });

  final ModuleChannel channel;
  final ShaperModel model;
  final MetalPalette palette;
  final Size size;
  final Animation<double> clock;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: channel,
      builder: (context, _) {
        final dim = !channel.on || channel.mute || (model.anySolo && !channel.solo);
        return Opacity(
          opacity: dim ? .5 : 1,
          child: NeuRaised(
            palette: palette,
            size: size,
            radius: 14,
            padding: const EdgeInsets.fromLTRB(12, 12, 12, 10),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  children: [
                    Expanded(
                      child: Text(
                        channel.title,
                        style: NeuType.chrome(17.5, palette.inkStrong, tracking: 1.4, w: FontWeight.w700),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                      ),
                    ),
                    TinyRoundButton(
                      palette: palette,
                      on: channel.on,
                      color: palette.ledOn,
                      size: 17,
                      onTap: channel.toggleOn,
                    ),
                    const SizedBox(width: 7),
                    TinyRoundButton(
                      palette: palette,
                      on: channel.solo,
                      color: palette.orange,
                      size: 17,
                      onTap: channel.toggleSolo,
                    ),
                    const SizedBox(width: 7),
                    TinyRoundButton(
                      palette: palette,
                      on: channel.mute,
                      color: palette.salmon,
                      size: 17,
                      onTap: channel.toggleMute,
                    ),
                  ],
                ),
                const SizedBox(height: 10),
                Expanded(
                  child: Row(
                    crossAxisAlignment: CrossAxisAlignment.center,
                    children: [
                      _KnobColumn(channel: channel, palette: palette),
                      const SizedBox(width: 10),
                      Expanded(
                        child: Column(
                          mainAxisAlignment: MainAxisAlignment.center,
                          crossAxisAlignment: CrossAxisAlignment.stretch,
                          children: [
                            _moduleWindow(channel, palette, size.width - 24 - 112 - 10),
                            const SizedBox(height: 12),
                            Row(
                              mainAxisAlignment: MainAxisAlignment.spaceBetween,
                              children: [
                                Expanded(
                                  child: MiniKnob(
                                    palette: palette,
                                    channel: channel,
                                    label: channel.labelA,
                                    value: channel.a,
                                    range: channel.aRange,
                                    unit: channel.aUnit,
                                    onChanged: channel.setA,
                                  ),
                                ),
                                Expanded(
                                  child: MiniKnob(
                                    palette: palette,
                                    channel: channel,
                                    label: channel.labelB,
                                    value: channel.b,
                                    range: channel.bRange,
                                    unit: channel.bUnit,
                                    onChanged: channel.setB,
                                  ),
                                ),
                              ],
                            ),
                          ],
                        ),
                      ),
                    ],
                  ),
                ),
                const SizedBox(height: 8),
                _CardFooter(channel: channel, palette: palette),
              ],
            ),
          ),
        );
      },
    );
  }

  Widget _moduleWindow(ModuleChannel c, MetalPalette p, double w) {
    return LayoutBuilder(
      // 参考图的显示窗是紧凑小窗，不随模块变宽而拉伸
      builder: (context, box) => _moduleWindowSized(c, p, math.min(box.maxWidth, 112)),
    );
  }

  Widget _moduleWindowSized(ModuleChannel c, MetalPalette p, double w) {
    switch (c.window) {
      case ModuleWindow.lcd:
        return NeuLcd(
          palette: p,
          value: c.lcdText ?? formatFreq(c.freq),
          unit: c.lcdUnit,
          topLabels: c.lcdLabels,
          accent: p.cyan,
          width: w,
          height: 62,
        );
      case ModuleWindow.lamp:
        final lampW = (w - 6) * .54;
        return Row(
          children: [
            NeuLamp(palette: p, color: p.orange, size: Size(lampW, 62)),
            const SizedBox(width: 6),
            NeuLcd(
              palette: p,
              value: c.lcdText ?? formatFreq(c.freq),
              unit: c.lcdUnit,
              topLabels: [c.lcdLabels[0]],
              accent: p.orange,
              width: w - 6 - lampW,
              height: 62,
              fontSize: 15,
            ),
          ],
        );
      case ModuleWindow.scope:
        return _MiniScope(
          palette: p,
          model: model,
          size: Size(w, 62),
          clock: clock,
        );
    }
  }
}

/// 卡片底部的两格事实：传输/编解码与丢帧。
class _CardFooter extends StatelessWidget {
  const _CardFooter({required this.channel, required this.palette});

  final ModuleChannel channel;
  final MetalPalette palette;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      height: 44,
      child: NeuWell(
        radius: 9,
        palette: palette,
        tint: palette.wellTint,
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 5),
        child: Row(
          children: [
            Expanded(child: _cell('路径', channel.infoA)),
            Container(width: 1, height: 22, color: palette.bevel.withValues(alpha: .35)),
            Expanded(child: _cell('丢帧', channel.infoB)),
          ],
        ),
      ),
    );
  }

  Widget _cell(String label, String value) {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 6),
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(label, style: NeuType.chrome(9.5, palette.inkFaint, tracking: .8)),
          const SizedBox(height: 1),
          Text(
            value,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: NeuType.numeric(11, palette.ink),
          ),
        ],
      ),
    );
  }
}

/// 模块窗内的迷你 EQ 曲线：与主屏同源，无流时画一条待机正弦。
class _MiniScope extends StatelessWidget {
  const _MiniScope({
    required this.palette,
    required this.model,
    required this.size,
    required this.clock,
  });

  final MetalPalette palette;
  final ShaperModel model;
  final Size size;
  final Animation<double> clock;

  @override
  Widget build(BuildContext context) {
    return NeuWell(
      size: size,
      radius: 8,
      palette: palette,
      deep: true,
      child: AnimatedBuilder(
        animation: clock,
        builder: (context, _) => CustomPaint(
          size: size,
          painter: _MiniScopePainter(
            palette: palette,
            curves: model.eqCurves,
            phase: clock.value,
          ),
        ),
      ),
    );
  }
}

class _MiniScopePainter extends CustomPainter {
  _MiniScopePainter({required this.palette, required this.curves, required this.phase});

  final MetalPalette palette;
  final List<EqCurve> curves;
  final double phase;

  @override
  void paint(Canvas canvas, Size size) {
    final plot = Rect.fromLTRB(5, 7, size.width - 5, size.height - 5);
    final mid = plot.center.dy;
    canvas.drawLine(
      Offset(plot.left, mid),
      Offset(plot.right, mid),
      Paint()..color = palette.inkFaint.withValues(alpha: .14),
    );
    if (curves.isEmpty) {
      final path = Path();
      for (var i = 0; i <= 40; i++) {
        final t = i / 40;
        final y = mid + math.sin((t * 2.6 + phase) * math.pi * 2) * plot.height * .18;
        if (i == 0) {
          path.moveTo(plot.left + t * plot.width, y);
        } else {
          path.lineTo(plot.left + t * plot.width, y);
        }
      }
      canvas.drawPath(
        path,
        Paint()
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1.4
          ..color = palette.inkFaint.withValues(alpha: .5),
      );
      return;
    }
    for (final curve in curves) {
      final bands = curve.bands;
      if (bands.length < 2) continue;
      final gain = math.pow(10, curve.gainDb / 20).toDouble().clamp(0.0, 1.6);
      final path = Path();
      for (var i = 0; i < bands.length; i++) {
        final x = plot.left + (i / (bands.length - 1)) * plot.width;
        final level = (bands[i] * gain).clamp(0.0, 1.0);
        final y = plot.bottom - level * plot.height * .86;
        if (i == 0) {
          path.moveTo(x, y);
        } else {
          path.lineTo(x, y);
        }
      }
      canvas.drawPath(
        path,
        Paint()
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1.2
          ..strokeJoin = StrokeJoin.round
          ..color = curve.color.withValues(alpha: curve.active ? .9 : .35),
      );
    }
  }

  @override
  bool shouldRepaint(_MiniScopePainter old) =>
      old.phase != phase || old.curves != curves || old.palette != palette;
}

class _KnobColumn extends StatelessWidget {
  const _KnobColumn({required this.channel, required this.palette});
  final ModuleChannel channel;
  final MetalPalette palette;

  @override
  Widget build(BuildContext context) {
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        NeuKnob(
          palette: palette,
          size: 112,
          value: channel.amount,
          range: channel.amountRange,
          accentColor: channel.accentFor(palette),
          onChanged: channel.setAmount,
        ),
        const SizedBox(height: 4),
        SizedBox(
          width: 112,
          child: Row(
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            children: [
              Text(
                channel.amountRange.min.toInt().toString(),
                style: NeuType.chrome(10, palette.inkFaint, tracking: .6),
              ),
              Text(
                channel.amountRange.min < 0
                    ? '+${channel.amountRange.max.toInt()}'
                    : channel.amountRange.max.toInt().toString(),
                style: NeuType.chrome(10, palette.inkFaint, tracking: .6),
              ),
            ],
          ),
        ),
        const SizedBox(height: 6),
        NeuValueField(
          palette: palette,
          value: channel.amount,
          range: channel.amountRange,
          onChanged: channel.setAmount,
          width: 76,
          height: 26,
          unit: channel.amountUnit,
          accent: channel.accentFor(palette),
          fontSize: 12.5,
        ),
      ],
    );
  }
}

/// 小旋钮 + 上方标签 + 下方可输入数值
class MiniKnob extends StatelessWidget {
  const MiniKnob({
    super.key,
    required this.palette,
    required this.channel,
    required this.label,
    required this.value,
    required this.range,
    required this.unit,
    required this.onChanged,
  });

  final MetalPalette palette;
  final ModuleChannel channel;
  final String label;
  final double value;
  final NeuRange range;
  final String unit;
  final ValueChanged<double> onChanged;

  @override
  Widget build(BuildContext context) {
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        SizedBox(
          width: 42,
          child: FittedBox(
            fit: BoxFit.scaleDown,
            child: Text(label, style: NeuType.chrome(10, palette.inkFaint, tracking: .6, w: FontWeight.w700)),
          ),
        ),
        const SizedBox(height: 4),
        NeuKnob(
          palette: palette,
          size: 40,
          value: value,
          range: range,
          accentColor: channel.accentFor(palette),
          onChanged: onChanged,
        ),
        const SizedBox(height: 4),
        NeuValueField(
          palette: palette,
          value: value,
          range: range,
          onChanged: onChanged,
          width: 42,
          height: 22,
          unit: unit,
          accent: palette.ink,
          digits: range.max >= 100 ? 0 : 1,
          fontSize: 11,
        ),
      ],
    );
  }
}

/// 底栏：HP —— LP、DRY/WET、BYPASS
class BottomStrip extends StatelessWidget {
  const BottomStrip({super.key, required this.model, required this.palette, required this.width});

  final ShaperModel model;
  final MetalPalette palette;
  final double width;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: model,
      builder: (context, _) => NeuWell(
        size: Size(width, 44),
        radius: 12,
        palette: palette,
        tint: palette.stripTint,
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
        child: Row(
          children: [
            Text('缓冲', style: NeuType.chrome(10, palette.ink, tracking: 1.2, w: FontWeight.w700)),
            const SizedBox(width: 6),
            Expanded(
              flex: 3,
              child: LayoutBuilder(
                builder: (context, box) => NeuSlider(
                  palette: palette,
                  value: model.hp,
                  range: ShaperModel.hpRange,
                  accentColor: palette.cyan,
                  width: box.maxWidth,
                  height: 22,
                  onChanged: model.setHp,
                ),
              ),
            ),
            const SizedBox(width: 6),
            Text('增益', style: NeuType.chrome(10, palette.ink, tracking: 1.2, w: FontWeight.w700)),
            const SizedBox(width: 8),
            NeuValueField(
              palette: palette,
              value: model.lp,
              range: ShaperModel.lpRange,
              onChanged: model.setLp,
              width: 50,
              height: 22,
              unit: 'dB',
              digits: 1,
              fontSize: 10.5,
            ),
            const SizedBox(width: 12),
            Text('DRY', style: NeuType.chrome(10, palette.inkFaint, tracking: 1.2, w: FontWeight.w700)),
            const SizedBox(width: 6),
            Expanded(
              flex: 2,
              child: LayoutBuilder(
                builder: (context, box) => NeuSlider(
                  palette: palette,
                  value: model.dry,
                  range: ShaperModel.dryWetRange,
                  accentColor: palette.ink,
                  width: box.maxWidth,
                  height: 22,
                  onChanged: model.setDry,
                ),
              ),
            ),
            const SizedBox(width: 6),
            Text('WET', style: NeuType.chrome(10, palette.ink, tracking: 1.2, w: FontWeight.w700)),
            const SizedBox(width: 12),
            NeuPill(
              palette: palette,
              label: 'BYPASS',
              height: 28,
              fontSize: 10.5,
              led: true,
              active: model.bypass,
              accent: palette.salmon,
              onTap: model.toggleBypass,
            ),
          ],
        ),
      ),
    );
  }
}
