import 'dart:math' as math;

import 'package:flutter/material.dart';

import 'controls.dart';
import 'tokens.dart';

/// 参考图 1 的三块面板：深色示波窗、同心点阵格栅、深色标题条。

/// 左屏：网格 + 透视线 + 波形（Siren 语义 = 主输出电平/波形）
class TantrumScope extends StatelessWidget {
  const TantrumScope({
    super.key,
    required this.m,
    required this.waveform,
    required this.title,
    required this.subtitle,
  });

  final TantrumMetrics m;
  final List<double> waveform;
  final String title;
  final String subtitle;

  @override
  Widget build(BuildContext context) {
    return CustomPaint(
      painter: _ScopePainter(waveform),
      child: Padding(
        padding: EdgeInsets.only(left: 9 * m.scale, top: 7 * m.scale),
        child: Row(
          children: [
            Text(title,
                style: TantrumTokens.caption(m.scale, color: TantrumTokens.inkOnDarkFaint)),
            SizedBox(width: 6 * m.scale),
            Text(subtitle, style: TantrumTokens.caption(m.scale, color: TantrumTokens.inkOnDark)),
          ],
        ),
      ),
    );
  }
}

class _ScopePainter extends CustomPainter {
  const _ScopePainter(this.waveform);

  final List<double> waveform;

  @override
  void paint(Canvas canvas, Size size) {
    final rect = Offset.zero & size;
    final clip = RRect.fromRectAndRadius(rect, Radius.circular(7));
    canvas.save();
    canvas.clipRRect(clip);
    canvas.drawRect(rect, Paint()..shader = const LinearGradient(
          begin: Alignment.topCenter,
          end: Alignment.bottomCenter,
          colors: [TantrumTokens.screenTop, TantrumTokens.screenBottom],
        ).createShader(rect));

    final hairline = Paint()
      ..color = TantrumTokens.screenGrid
      ..strokeWidth = 1;
    // 上半区：方格
    for (var i = 1; i < 8; i++) {
      final x = size.width * i / 8;
      canvas.drawLine(Offset(x, 0), Offset(x, size.height * 0.58), hairline);
    }
    for (var i = 1; i < 4; i++) {
      final y = size.height * 0.58 * i / 4;
      canvas.drawLine(Offset(0, y), Offset(size.width, y), hairline);
    }
    // 下半区：透视线地板（图1 的屏底网格）
    final floor = Paint()
      ..color = TantrumTokens.screenGridFloor
      ..strokeWidth = 1;
    final horizon = size.height * 0.58;
    for (var i = 0; i <= 6; i++) {
      final t = i / 6;
      final topX = size.width * (0.18 + 0.64 * t);
      final bottomX = size.width * (-0.35 + 1.7 * t);
      canvas.drawLine(Offset(topX, horizon), Offset(bottomX, size.height), floor);
    }
    for (var i = 1; i <= 4; i++) {
      final y = horizon + (size.height - horizon) * math.pow(i / 4, 1.9).toDouble();
      canvas.drawLine(Offset(0, y), Offset(size.width, y), floor);
    }

    // 波形
    if (waveform.isNotEmpty) {
      final mid = horizon * 0.52;
      final amp = horizon * 0.34;
      final path = Path()..moveTo(0, mid);
      for (var i = 0; i < waveform.length; i++) {
        path.lineTo(size.width * i / (waveform.length - 1), mid - waveform[i] * amp);
      }
      final stroke = Paint()
        ..color = TantrumTokens.screenInk
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.6
        ..strokeJoin = StrokeJoin.round;
      // 波形下面只留一条极淡的暗线做厚度，别用 drawShadow：
      // 方波的竖直段会被糊成一大片黑雾。
      canvas.drawPath(
        path.shift(const Offset(0, 1.4)),
        Paint()
          ..color = const Color(0x40000000)
          ..style = PaintingStyle.stroke
          ..strokeWidth = 2.4,
      );
      canvas.drawPath(path, stroke);
    }
    canvas.restore();
    canvas.drawRRect(clip, Paint()..color = const Color(0x33000000)..style = PaintingStyle.stroke..strokeWidth = 1);
  }

  @override
  bool shouldRepaint(_ScopePainter old) => old.waveform != waveform;
}

/// 右格栅：同心点阵 + 左上红扇区 + 中心圆（图1 的喇叭罩）
class TantrumGrille extends StatelessWidget {
  const TantrumGrille({super.key, required this.m, required this.centerLabel});

  final TantrumMetrics m;
  final String centerLabel;

  @override
  Widget build(BuildContext context) {
    return CustomPaint(
      painter: _GrillePainter(),
      child: Center(
        child: Text(centerLabel,
            style: TantrumTokens.caption(m.scale, color: TantrumTokens.inkFaint)),
      ),
    );
  }
}

class _GrillePainter extends CustomPainter {
  @override
  void paint(Canvas canvas, Size size) {
    final rect = Offset.zero & size;
    final clip = RRect.fromRectAndRadius(rect, Radius.circular(7));
    canvas.save();
    canvas.clipRRect(clip);
    canvas.drawRect(rect, Paint()..color = Colors.white);
    final center = Offset(size.width * 0.5, size.height * 0.54);
    final maxR = size.width * 0.46;
    var ring = 1;
    for (var r = maxR * 0.2; r < maxR; r += maxR * 0.08) {
      final count = (2 * math.pi * r / (maxR * 0.07)).ceil();
      for (var i = 0; i < count; i++) {
        final angle = 2 * math.pi * i / count - math.pi / 2;
        final p = center + Offset(math.cos(angle), math.sin(angle)) * r;
        // 图1：左上扇区的点是红的（画布 y 轴向下，π=左、1.5π=上）
        final inWedge = angle > math.pi * 1.02 && angle < math.pi * 1.42;
        canvas.drawCircle(
          p,
          size.width * 0.0048 + ring * 0.03,
          Paint()
            ..color = inWedge
                ? TantrumTokens.accent.withValues(alpha: 0.55)
                : TantrumTokens.tickIdle.withValues(alpha: 0.85),
        );
      }
      ring++;
    }
    // 中心圆 + 四个刻度
    final core = size.width * 0.055;
    canvas.drawCircle(center, core, Paint()..color = Colors.white);
    canvas.drawCircle(
      center,
      core,
      Paint()
        ..color = TantrumTokens.knobEdge
        ..style = PaintingStyle.stroke,
    );
    final tick = Paint()
      ..color = TantrumTokens.ink
      ..strokeWidth = 1.4
      ..strokeCap = StrokeCap.round;
    for (var i = 0; i < 4; i++) {
      final angle = i * math.pi / 2;
      final dir = Offset(math.cos(angle), math.sin(angle));
      canvas.drawLine(center + dir * (core * 1.35), center + dir * (core * 1.95), tick);
    }
    canvas.restore();
    canvas.drawRRect(
      clip,
      Paint()
        ..color = TantrumTokens.knobEdge
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1,
    );
  }

  @override
  bool shouldRepaint(covariant CustomPainter oldDelegate) => false;
}

/// EQ 图窗（图2 中间卡片：网格 + 虚线 0 dB + 频谱灰区 + 橙色曲线）
class TantrumEqGraph extends StatelessWidget {
  const TantrumEqGraph({super.key, required this.m, required this.curve, required this.spectrum});

  final TantrumMetrics m;
  final List<double> curve;
  final List<double> spectrum;

  @override
  Widget build(BuildContext context) {
    return CustomPaint(painter: _EqPainter(curve, spectrum));
  }
}

class _EqPainter extends CustomPainter {
  const _EqPainter(this.curve, this.spectrum);

  final List<double> curve;
  final List<double> spectrum;

  @override
  void paint(Canvas canvas, Size size) {
    final rect = Offset.zero & size;
    final clip = RRect.fromRectAndRadius(rect, Radius.circular(6));
    canvas.save();
    canvas.clipRRect(clip);
    canvas.drawRect(rect, Paint()..shader = const LinearGradient(
          begin: Alignment.topCenter,
          end: Alignment.bottomCenter,
          colors: [TantrumTokens.wellTop, TantrumTokens.wellBottom],
        ).createShader(rect));

    final grid = Paint()
      ..color = TantrumTokens.tickIdle
      ..strokeWidth = 1;
    for (var i = 1; i < 5; i++) {
      final x = size.width * i / 5;
      canvas.drawLine(Offset(x, 0), Offset(x, size.height), grid);
    }
    for (var i = 1; i < 4; i++) {
      final y = size.height * i / 4;
      canvas.drawLine(Offset(0, y), Offset(size.width, y), grid);
    }
    // 0 dB 虚线
    final zero = Paint()
      ..color = TantrumTokens.wellInk
      ..strokeWidth = 1;
    final mid = size.height * 0.42;
    for (var x = 0.0; x < size.width; x += 5) {
      canvas.drawLine(Offset(x, mid), Offset(math.min(x + 2.5, size.width), mid), zero);
    }

    // 频谱灰区
    if (spectrum.isNotEmpty) {
      final path = Path()..moveTo(0, size.height);
      for (var i = 0; i < spectrum.length; i++) {
        path.lineTo(size.width * i / (spectrum.length - 1),
            size.height - spectrum[i] * size.height * 0.85);
      }
      path.lineTo(size.width, size.height);
      canvas.drawPath(path, Paint()..color = TantrumTokens.tickIdle.withValues(alpha: 0.55));
    }
    // 橙色曲线（±12 dB 映射到面板高度）
    if (curve.length > 1) {
      final path = Path();
      for (var i = 0; i < curve.length; i++) {
        final x = size.width * i / (curve.length - 1);
        final y = mid - curve[i] * size.height * 0.34;
        i == 0 ? path.moveTo(x, y) : path.lineTo(x, y);
      }
      canvas.drawPath(
        path,
        Paint()
          ..color = TantrumTokens.curve
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1.6
          ..strokeJoin = StrokeJoin.round,
      );
    }
    canvas.restore();
    canvas.drawRRect(
      clip,
      Paint()
        ..color = TantrumTokens.knobEdge
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1,
    );
  }

  @override
  bool shouldRepaint(_EqPainter old) => old.curve != curve || old.spectrum != spectrum;
}

/// EQ 图窗 + 四推子（图2 中间卡片的整体）
class TantrumEqBank extends StatelessWidget {
  const TantrumEqBank({
    super.key,
    required this.m,
    required this.bands,
    required this.curve,
    required this.spectrum,
    this.onBand,
  });

  final TantrumMetrics m;
  final List<double> bands;
  final List<double> curve;
  final List<double> spectrum;
  final void Function(int index, double value)? onBand;

  static const _labels = ['低频', '中频', '高频', '空气'];

  @override
  Widget build(BuildContext context) {
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        SizedBox(
          height: m.bodyHeight * 0.42,
          width: double.infinity,
          child: TantrumEqGraph(m: m, curve: curve, spectrum: spectrum),
        ),
        SizedBox(height: 8 * m.scale),
        Row(
          mainAxisAlignment: MainAxisAlignment.spaceEvenly,
          children: [
            for (var i = 0; i < bands.length; i++)
              TantrumFader(
                value: bands[i],
                m: m,
                label: _labels[i],
                onChanged: onBand == null ? null : (v) => onBand!(i, v),
              ),
          ],
        ),
      ],
    );
  }
}

/// 深色标题条：品牌字 + 预设切换 + 齿轮 + 电源（图1 顶栏）
class TantrumHeader extends StatelessWidget {
  const TantrumHeader({
    super.key,
    required this.m,
    required this.brand,
    required this.preset,
    this.onPrev,
    this.onNext,
    this.onGear,
    this.onPower,
    this.powerOn = true,
  });

  final TantrumMetrics m;
  final String brand;
  final String preset;
  final VoidCallback? onPrev;
  final VoidCallback? onNext;
  final VoidCallback? onGear;
  final VoidCallback? onPower;
  final bool powerOn;

  @override
  Widget build(BuildContext context) {
    return Container(
      height: m.headerHeight,
      padding: EdgeInsets.symmetric(horizontal: 10 * m.scale),
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(7 * m.scale),
        gradient: const LinearGradient(
          begin: Alignment.topCenter,
          end: Alignment.bottomCenter,
          colors: [TantrumTokens.headerTop, TantrumTokens.headerBottom],
        ),
        boxShadow: [
          BoxShadow(
            color: TantrumTokens.panelShadow,
            blurRadius: 4 * m.scale,
            offset: Offset(0, 1.5 * m.scale),
          ),
        ],
      ),
      child: Row(
        children: [
          Text(brand, style: TantrumTokens.brand(m.scale)),
          SizedBox(width: 6 * m.scale),
          const _BrandGlyph(),
          const Spacer(),
          Icon(Icons.chevron_left_rounded, size: 14 * m.scale, color: TantrumTokens.inkOnDark)
              .gesture(onPrev),
          SizedBox(width: 4 * m.scale),
          Text(preset, style: TantrumTokens.preset(m.scale)),
          SizedBox(width: 4 * m.scale),
          Icon(Icons.chevron_right_rounded, size: 14 * m.scale, color: TantrumTokens.inkOnDark)
              .gesture(onNext),
          SizedBox(width: 10 * m.scale),
          TantrumRoundButton(
            m: m,
            icon: Icons.settings_rounded,
            onDark: true,
            onTap: onGear,
          ),
          SizedBox(width: 6 * m.scale),
          TantrumRoundButton(
            m: m,
            icon: Icons.power_settings_new_rounded,
            onDark: true,
            active: powerOn,
            onTap: onPower,
          ),
        ],
      ),
    );
  }
}

extension on Widget {
  Widget gesture(VoidCallback? tap) =>
      GestureDetector(onTap: tap, behavior: HitTestBehavior.opaque, child: this);
}

/// 品牌字右侧的小波形丝印（图1 的 TANTRUM···）
class _BrandGlyph extends StatelessWidget {
  const _BrandGlyph();

  @override
  Widget build(BuildContext context) {
    return CustomPaint(
      size: const Size(26, 10),
      painter: _GlyphPainter(),
    );
  }
}

class _GlyphPainter extends CustomPainter {
  @override
  void paint(Canvas canvas, Size size) {
    final path = Path()
      ..moveTo(0, size.height * 0.5)
      ..lineTo(size.width * 0.18, size.height * 0.5)
      ..lineTo(size.width * 0.28, size.height * 0.12)
      ..lineTo(size.width * 0.42, size.height * 0.88)
      ..lineTo(size.width * 0.56, size.height * 0.3)
      ..lineTo(size.width * 0.7, size.height * 0.7)
      ..lineTo(size.width, size.height * 0.5);
    canvas.drawPath(
      path,
      Paint()
        ..color = TantrumTokens.inkOnDark
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.2
        ..strokeJoin = StrokeJoin.round,
    );
  }

  @override
  bool shouldRepaint(covariant CustomPainter oldDelegate) => false;
}

/// 面板外壳：桌面渐变 + 白塑面 + 顶边高光（图1 的机箱）
class TantrumShell extends StatelessWidget {
  const TantrumShell({super.key, required this.width, required this.child});

  final double width;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final m = TantrumMetrics(width);
    return Container(
      decoration: const BoxDecoration(
        gradient: LinearGradient(
          begin: Alignment.topCenter,
          end: Alignment.bottomCenter,
          colors: [TantrumTokens.deskTop, TantrumTokens.deskBottom],
        ),
      ),
      alignment: Alignment.center,
      child: SizedBox(
        width: width,
        height: m.height,
        child: DecoratedBox(
          decoration: BoxDecoration(
            borderRadius: BorderRadius.circular(m.radius),
            gradient: const LinearGradient(
              begin: Alignment.topCenter,
              end: Alignment.bottomCenter,
              colors: [TantrumTokens.panelTop, TantrumTokens.panelBottom],
            ),
            boxShadow: [
              BoxShadow(
                color: TantrumTokens.panelShadow,
                blurRadius: 26 * m.scale,
                offset: Offset(0, 10 * m.scale),
              ),
            ],
          ),
          child: ClipRRect(
            borderRadius: BorderRadius.circular(m.radius),
            child: Stack(
              children: [
                Positioned(
                  left: 0,
                  right: 0,
                  top: 0,
                  height: 1.4,
                  child: DecoratedBox(decoration: const BoxDecoration(color: TantrumTokens.panelBevel)),
                ),
                child,
              ],
            ),
          ),
        ),
      ),
    );
  }
}
