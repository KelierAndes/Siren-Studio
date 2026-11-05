import 'dart:math' as math;

import 'package:flutter/material.dart';

import 'relief.dart';
import 'tokens.dart';

/// 参考图 2 的控件 token 实现：点阵刻度环旋钮、帽式推子、分段胶囊、
/// ADV 开关、圆形电源/齿轮钮、LED、上下箭头组。控件只回抛值，不碰数据源。

const double _knobStart = math.pi * 0.75; // 7 点钟方向（画布 y 轴向下）
const double _knobSweep = math.pi * 1.5; // 共 270°

/// 旋钮：value 归一化 0–1。
class TantrumKnob extends StatelessWidget {
  const TantrumKnob({
    super.key,
    required this.value,
    required this.m,
    this.onChanged,
  });

  final double value;
  final TantrumMetrics m;
  final ValueChanged<double>? onChanged;

  @override
  Widget build(BuildContext context) {
    final side = m.knobRingRadius * 2 + 4 * m.scale;
    return GestureDetector(
      onPanUpdate: onChanged == null
          ? null
          : (d) => onChanged!((value - d.delta.dy / (140 * m.scale)).clamp(0.0, 1.0)),
      child: SizedBox(
        width: side,
        height: m.knobBoxHeight,
        child: CustomPaint(painter: _KnobPainter(value, m)),
      ),
    );
  }
}

class _KnobPainter extends CustomPainter {
  const _KnobPainter(this.value, this.m);

  final double value;
  final TantrumMetrics m;

  @override
  void paint(Canvas canvas, Size size) {
    final center = size.center(Offset.zero);
    final r = m.knobDiameter / 2;
    final faceRect = Rect.fromCircle(center: center, radius: r);
    final ticks = m.tickCount.toInt();
    final active = (value * (ticks - 1)).round();

    for (var i = 0; i < ticks; i++) {
      final angle = _knobStart + i / (ticks - 1) * _knobSweep;
      final painted = i <= active;
      final at = center + Offset(math.cos(angle), math.sin(angle)) * m.knobRingRadius;
      // 刻度点是铣进面板的小坑：暗点下面再补一条回白
      canvas.drawCircle(
        at,
        m.tickRadius * (painted ? 1.0 : 0.78),
        Paint()..color = painted ? TantrumTokens.tickActive : TantrumTokens.tickIdle,
      );
      canvas.drawCircle(
        at + Offset(0, 0.6 * m.scale),
        m.tickRadius * (painted ? 0.62 : 0.5),
        Paint()..color = TantrumTokens.rimLight.withValues(alpha: 0.4),
      );
    }

    // 本体：投影 → 侧壁 → 顶面（球面渐变）→ 顶面压边 → 斜边 → 镜面高光。
    // 只画一层线性渐变的圆面会读成"一张白纸"，塑料旋钮靠的是侧壁和顶面的两级落差。
    paintRaiseCircle(canvas, center, r, m.scale, depth: 1.2);
    canvas.drawCircle(
      center,
      r,
      Paint()
        ..shader = faceShader(
          faceRect,
          const [Color(0xFFF4F6F9), Color(0xFFC2CAD5)],
          light: const Alignment(-0.45, -1),
        ),
    );
    final top = r - 4.6 * m.scale;
    canvas.save();
    canvas.clipRRect(
        RRect.fromRectAndRadius(Rect.fromCircle(center: center, radius: top),
            Radius.circular(top)));
    canvas.drawCircle(
      center,
      top,
      Paint()
        ..shader = RadialGradient(
          center: const Alignment(-0.42, -0.5),
          radius: 1.45,
          colors: const [
            Color(0xFFFFFFFF),
            TantrumTokens.knobBottom,
            TantrumTokens.knobShade,
          ],
          stops: const [0.0, 0.52, 1.0],
        ).createShader(Rect.fromCircle(center: center, radius: top)),
    );
    canvas.restore();
    // 顶面外沿的一圈压边：上缘受光、下缘投影，顶面才是"盖"在侧壁上的
    canvas.drawCircle(
      center,
      top,
      Paint()
        ..color = TantrumTokens.castDark
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.6 * m.scale,
    );
    canvas.drawArc(
      Rect.fromCircle(center: center, radius: top - 0.9 * m.scale),
      math.pi * 0.95,
      math.pi * 1.1,
      false,
      Paint()
        ..color = TantrumTokens.rimLight
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.2 * m.scale,
    );
    paintBevel(canvas, RRect.fromRectAndRadius(faceRect, Radius.circular(r)), m.scale);
    // 顶面左上的一道镜面高光，把"圆柱"和"圆片"区分开
    canvas.drawArc(
      Rect.fromCircle(center: center, radius: top - 6 * m.scale),
      math.pi * 1.06,
      math.pi * 0.62,
      false,
      Paint()
        ..color = const Color(0x99FFFFFF)
        ..style = PaintingStyle.stroke
        ..strokeWidth = 3.2 * m.scale
        ..strokeCap = StrokeCap.round
        ..maskFilter = MaskFilter.blur(BlurStyle.normal, 1.8 * m.scale),
    );

    // 红色数值弧（图1）：刻度环内侧一圈。
    // 未给量的轨道用中性灰——淡红轨道看起来像印刷残次。
    final arcR = m.knobRingRadius - 3.4 * m.scale;
    final arcRect = Rect.fromCircle(center: center, radius: arcR);
    final end = _knobStart + value * _knobSweep;
    canvas.drawArc(
      arcRect,
      _knobStart,
      _knobSweep,
      false,
      Paint()
        ..color = TantrumTokens.tickIdle.withValues(alpha: 0.55)
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.8 * m.scale,
    );
    if (value > 0.004) {
      canvas.drawArc(
        arcRect,
        _knobStart,
        end - _knobStart,
        false,
        Paint()
          ..color = TantrumTokens.accent.withValues(alpha: 0.18)
          ..style = PaintingStyle.stroke
          ..strokeWidth = 3.4 * m.scale
          ..maskFilter = MaskFilter.blur(BlurStyle.normal, 1.2 * m.scale),
      );
      canvas.drawArc(
        arcRect,
        _knobStart,
        end - _knobStart,
        false,
        Paint()
          ..color = TantrumTokens.accent
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1.8 * m.scale
          ..strokeCap = StrokeCap.round,
      );
    }

    // 指示槽：亮边在下、暗槽在上，读起来才是"刻出来的"
    final angle = _knobStart + value * _knobSweep;
    final mid = center + Offset(math.cos(angle), math.sin(angle)) * m.knobNotchRadius;
    canvas.save();
    canvas.translate(mid.dx, mid.dy);
    canvas.rotate(angle + math.pi / 2);
    final slot = RRect.fromRectAndRadius(
      Rect.fromCenter(center: Offset.zero, width: m.knobNotchWidth, height: m.knobNotchLength),
      Radius.circular(m.knobNotchWidth / 2),
    );
    canvas.drawRRect(slot.shift(Offset(0.7 * m.scale, 0.8 * m.scale)),
        Paint()..color = TantrumTokens.rimLight);
    canvas.drawRRect(slot, Paint()..color = TantrumTokens.notch);
    canvas.restore();
  }

  @override
  bool shouldRepaint(_KnobPainter old) => old.value != value || old.m.scale != m.scale;
}

/// 帽式推子（图2：槽宽 5、帽 26×13）
class TantrumFader extends StatelessWidget {
  const TantrumFader({
    super.key,
    required this.value,
    required this.m,
    required this.label,
    this.onChanged,
  });

  final double value;
  final TantrumMetrics m;
  final String label;
  final ValueChanged<double>? onChanged;

  @override
  Widget build(BuildContext context) {
    final travel = m.faderTravel;
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        SizedBox(
          width: m.faderCapWidth + 6 * m.scale,
          height: travel + 10 * m.scale,
          child: GestureDetector(
            onPanUpdate: onChanged == null
                ? null
                : (d) => onChanged!((value + d.delta.dy / (travel * 0.8)).clamp(0.0, 1.0)),
            child: CustomPaint(painter: _FaderPainter(value, m)),
          ),
        ),
        SizedBox(height: 5 * m.scale),
        Text(label, style: TantrumTokens.caption(m.scale, color: TantrumTokens.ink)),
      ],
    );
  }
}

class _FaderPainter extends CustomPainter {
  const _FaderPainter(this.value, this.m);

  final double value;
  final TantrumMetrics m;

  @override
  void paint(Canvas canvas, Size size) {
    final cx = size.width / 2;
    final top = 5 * m.scale;
    final bottom = size.height - 5 * m.scale;
    final slot = RRect.fromRectAndRadius(
      Rect.fromLTWH(cx - m.faderSlotWidth / 2, top, m.faderSlotWidth, bottom - top),
      Radius.circular(m.faderSlotWidth / 2),
    );
    // 槽是凹进去的：内阴影压在左上，槽底比面板暗两档，
    // 中间再留一条更暗的沟——浅灰单色槽看起来像画上去的白条。
    canvas.drawRRect(
      slot,
      Paint()
        ..shader = faceShader(
          slot.outerRect,
          const [TantrumTokens.meterFloorDark, Color(0xFFAEB6C2)],
          light: const Alignment(-0.4, -1),
        ),
    );
    paintRecess(canvas, slot, m.scale, depth: 1.1);
    canvas.drawLine(
      Offset(cx, top + 2 * m.scale),
      Offset(cx, bottom - 2 * m.scale),
      Paint()
        ..color = const Color(0x2E7A8698)
        ..strokeWidth = 1.2 * m.scale,
    );
    // 左侧刻度梯：推子的位置要能读出来，不然只是一块会动的白塑料
    final tickX = cx - m.faderSlotWidth / 2 - 2.2 * m.scale;
    for (var i = 0; i <= 4; i++) {
      final y = bottom - (bottom - top) * i / 4;
      final len = i == 2 ? 3.6 * m.scale : 2.2 * m.scale;
      canvas.drawLine(
        Offset(tickX - len, y),
        Offset(tickX, y),
        Paint()
          ..color = i == 2 ? TantrumTokens.ink : TantrumTokens.tickIdle
          ..strokeWidth = 1,
      );
    }
    final capCenter = bottom - (bottom - top) * value;
    final cap = Rect.fromCenter(
      center: Offset(cx, capCenter),
      width: m.faderCapWidth,
      height: m.faderCapHeight,
    );
    final capShape = RRect.fromRectAndRadius(cap, Radius.circular(4 * m.scale));
    paintRaise(canvas, capShape, m.scale, depth: 0.8);
    canvas.drawRRect(
      capShape,
      Paint()
        ..shader = faceShader(
          cap,
          const [TantrumTokens.capTop, TantrumTokens.capBottom],
          light: const Alignment(-0.3, -1),
        ),
    );
    paintBevel(canvas, capShape, m.scale, width: 0.9);
    // 帽上的握线：暗线 + 下缘回白
    canvas.drawLine(
      Offset(cap.left + 5 * m.scale, cap.center.dy + 0.8 * m.scale),
      Offset(cap.right - 5 * m.scale, cap.center.dy + 0.8 * m.scale),
      Paint()
        ..color = TantrumTokens.rimLight
        ..strokeWidth = 2 * m.scale
        ..strokeCap = StrokeCap.round,
    );
    canvas.drawLine(
      Offset(cap.left + 5 * m.scale, cap.center.dy),
      Offset(cap.right - 5 * m.scale, cap.center.dy),
      Paint()
        ..color = TantrumTokens.notch
        ..strokeWidth = 2 * m.scale
        ..strokeCap = StrokeCap.round,
    );
  }

  @override
  bool shouldRepaint(_FaderPainter old) => old.value != value || old.m.scale != m.scale;
}

/// 分段胶囊（图2：Fast / Slow / Auto）
class TantrumSegPill extends StatelessWidget {
  const TantrumSegPill({
    super.key,
    required this.items,
    required this.index,
    required this.m,
    this.onSelect,
  });

  final List<String> items;
  final int index;
  final TantrumMetrics m;
  final ValueChanged<int>? onSelect;

  @override
  Widget build(BuildContext context) {
    final width = m.segSegmentWidth * items.length + 6 * m.scale;
    return GestureDetector(
      onTapUp: onSelect == null
          ? null
          : (d) => onSelect!((((d.localPosition.dx - 3 * m.scale) / m.segSegmentWidth).floor())
              .clamp(0, items.length - 1)),
      child: SizedBox(
        width: width,
        height: m.segHeight,
        child: CustomPaint(painter: _SegPainter(items, index, m)),
      ),
    );
  }
}

class _SegPainter extends CustomPainter {
  const _SegPainter(this.items, this.index, this.m);

  final List<String> items;
  final int index;
  final TantrumMetrics m;

  @override
  void paint(Canvas canvas, Size size) {
    final pad = 3 * m.scale;
    final track = Rect.fromLTWH(pad, 0, size.width - pad * 2, size.height);
    final trackShape = RRect.fromRectAndRadius(track, Radius.circular(size.height / 2));
    // 轨道是沉进面板的槽：底色比面板暗一档，再压内阴影
    canvas.drawRRect(trackShape, Paint()..color = TantrumTokens.segTrack);
    paintRecess(canvas, trackShape, m.scale, depth: 0.9);
    final w = track.width / items.length;
    final active = Rect.fromLTWH(
      track.left + index * w + 1.5 * m.scale,
      1.5 * m.scale,
      w - 3 * m.scale,
      size.height - 3 * m.scale,
    );
    final activeShape = RRect.fromRectAndRadius(active, Radius.circular(active.height / 2));
    paintRaise(canvas, activeShape, m.scale, depth: 0.7);
    canvas.drawRRect(
      activeShape,
      Paint()
        ..shader = faceShader(active, const [Colors.white, TantrumTokens.capBottom],
            light: const Alignment(-0.3, -1)),
    );
    paintBevel(canvas, activeShape, m.scale, width: 0.8);
    for (var i = 0; i < items.length; i++) {
      final tp = TextPainter(
        text: TextSpan(
          text: items[i],
          style: TantrumTokens.caption(
            m.scale * 1.15,
            color: i == index ? TantrumTokens.inkStrong : TantrumTokens.inkFaint,
          ),
        ),
        textDirection: TextDirection.ltr,
      )..layout();
      tp.paint(
        canvas,
        Offset(track.left + i * w + (w - tp.width) / 2, (size.height - tp.height) / 2),
      );
    }
  }

  @override
  bool shouldRepaint(_SegPainter old) => old.index != index || old.m.scale != m.scale;
}

/// ADV 开关（图2：开 = 深色轨道，滑块在右）
class TantrumSwitch extends StatelessWidget {
  const TantrumSwitch({super.key, required this.on, required this.m, this.onChanged});

  final bool on;
  final TantrumMetrics m;
  final ValueChanged<bool>? onChanged;

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onTap: onChanged == null ? null : () => onChanged!(!on),
      child: SizedBox(
        width: m.switchWidth,
        height: m.switchHeight,
        child: CustomPaint(painter: _SwitchPainter(on, m)),
      ),
    );
  }
}

class _SwitchPainter extends CustomPainter {
  const _SwitchPainter(this.on, this.m);

  final bool on;
  final TantrumMetrics m;

  @override
  void paint(Canvas canvas, Size size) {
    final track = RRect.fromRectAndRadius(Offset.zero & size, Radius.circular(size.height / 2));
    // 轨道永远是沉槽：开=深色槽，关=浅色槽，靠内阴影而不是靠边框区分
    canvas.drawRRect(
      track,
      Paint()
        ..shader = faceShader(
          track.outerRect,
          on
              ? const [Color(0xFF2E333A), TantrumTokens.switchOn]
              : const [
                  TantrumTokens.meterFloorDark,
                  TantrumTokens.meterFloor,
                ],
          light: const Alignment(-0.2, -1),
        ),
    );
    paintRecess(canvas, track, m.scale, depth: 0.8);
    final r = size.height / 2 - 2 * m.scale;
    final center = Offset(on ? size.width - r - 2 * m.scale : r + 2 * m.scale, size.height / 2);
    paintRaiseCircle(canvas, center, r, m.scale, depth: 0.7);
    canvas.drawCircle(
      center,
      r,
      Paint()
        ..shader = faceShader(Rect.fromCircle(center: center, radius: r),
            const [Colors.white, TantrumTokens.capBottom], light: const Alignment(-0.4, -1)),
    );
    paintBevel(canvas,
        RRect.fromRectAndRadius(Rect.fromCircle(center: center, radius: r), Radius.circular(r)),
        m.scale,
        width: 0.8);
  }

  @override
  bool shouldRepaint(_SwitchPainter old) => old.on != on || old.m.scale != m.scale;
}

/// 圆形按钮：电源、齿轮。onDark 时只画图标（贴在深色标题条上）。
class TantrumRoundButton extends StatelessWidget {
  const TantrumRoundButton({
    super.key,
    required this.m,
    required this.icon,
    this.onDark = false,
    this.active = false,
    this.onTap,
  });

  final TantrumMetrics m;
  final IconData icon;
  final bool onDark;
  final bool active;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onTap: onTap,
      child: SizedBox(
        width: m.roundButton,
        height: m.roundButton,
        child: CustomPaint(painter: _RoundBtnPainter(icon, m, onDark, active)),
      ),
    );
  }
}

class _RoundBtnPainter extends CustomPainter {
  const _RoundBtnPainter(this.icon, this.m, this.onDark, this.active);

  final IconData icon;
  final TantrumMetrics m;
  final bool onDark;
  final bool active;

  @override
  void paint(Canvas canvas, Size size) {
    final center = size.center(Offset.zero);
    final r = size.width / 2;
    final face = RRect.fromRectAndRadius(Rect.fromCircle(center: center, radius: r),
        Radius.circular(r));
    if (!onDark) {
      paintRaiseCircle(canvas, center, r, m.scale, depth: 0.8);
      canvas.drawCircle(
        center,
        r,
        Paint()
          ..shader = faceShader(Rect.fromCircle(center: center, radius: r),
              const [Colors.white, TantrumTokens.capBottom], light: const Alignment(-0.4, -1)),
      );
      paintBevel(canvas, face, m.scale, width: 0.9);
    } else {
      // 深色标题条上的圆钮是凹进去的按钮井：井底比条面暗一档，
      // 再补一圈下缘回白，不然只是图标前面多了一块半透明灰。
      canvas.drawCircle(
        center,
        r,
        Paint()
          ..shader = faceShader(Rect.fromCircle(center: center, radius: r),
              const [Color(0xFF2A2F36), Color(0xFF3F454D)], light: const Alignment(-0.3, -1)),
      );
      paintRecessCircle(canvas, center, r, m.scale, depth: 0.9);
      canvas.drawCircle(
        center,
        r - 0.6 * m.scale,
        Paint()
          ..color = const Color(0x1FFFFFFF)
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1,
      );
    }
    if (active && onDark) {
      // 通电的井里泛一层红光：图标本身是红的，但只有面光才像真的亮着
      canvas.drawCircle(
        center,
        r * 0.78,
        Paint()
          ..color = TantrumTokens.accentSoft
          ..maskFilter = MaskFilter.blur(BlurStyle.normal, r * 0.45),
      );
    }
    final tp = TextPainter(
      text: TextSpan(
        text: String.fromCharCode(icon.codePoint),
        style: TextStyle(
          fontFamily: icon.fontFamily,
          package: icon.fontPackage,
          fontSize: r * 1.15,
          color: onDark
              ? (active ? TantrumTokens.accent : TantrumTokens.inkOnDark)
              : (active ? TantrumTokens.accent : TantrumTokens.ink),
        ),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    tp.paint(canvas, center - Offset(tp.width / 2, tp.height / 2));
  }

  @override
  bool shouldRepaint(_RoundBtnPainter old) =>
      old.icon != icon || old.onDark != onDark || old.active != active || old.m.scale != m.scale;
}

/// LED（图1 旋钮之间那颗红点）
class TantrumLed extends StatelessWidget {
  const TantrumLed({super.key, required this.on, required this.m});

  final bool on;
  final TantrumMetrics m;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: m.ledDiameter,
      height: m.ledDiameter,
      child: CustomPaint(painter: _LedPainter(on, m)),
    );
  }
}

class _LedPainter extends CustomPainter {
  const _LedPainter(this.on, this.m);

  final bool on;
  final TantrumMetrics m;

  @override
  void paint(Canvas canvas, Size size) {
    final center = size.center(Offset.zero);
    final r = size.width / 2;
    // 灯座是沉孔，灯珠在孔里发光：先内阴影，再发光，最后高光点
    canvas.drawCircle(center, r, Paint()..color = TantrumTokens.ledOff);
    paintRecessCircle(canvas, center, r, m.scale, depth: 0.6);
    if (on) {
      canvas.drawCircle(
        center,
        r * 1.9,
        Paint()
          ..color = TantrumTokens.accentSoft
          ..maskFilter = MaskFilter.blur(BlurStyle.normal, r * 0.9),
      );
    }
    canvas.drawCircle(
      center,
      r - 1.2,
      Paint()
        ..shader = RadialGradient(
          center: const Alignment(-0.35, -0.4),
          colors: on
              ? const [Color(0xFFFF8B6E), TantrumTokens.ledOn, Color(0xFF9E2413)]
              : const [Color(0xFFEFF2F6), TantrumTokens.ledOff, Color(0xFFB9C1CC)],
          stops: const [0.15, 0.62, 1.0],
        ).createShader(Rect.fromCircle(center: center, radius: r - 1.2)),
    );
  }

  @override
  bool shouldRepaint(_LedPainter old) => old.on != on || old.m.scale != m.scale;
}

/// 混音/直通胶囊：凸出 = 混音生效，按下去（沉进面板）= 直通。
/// Flutter 的 BoxShadow 画不出内阴影，所以这里走 relief 原语而不是装饰。
class TantrumCapsule extends StatelessWidget {
  const TantrumCapsule({
    super.key,
    required this.label,
    required this.raised,
    required this.m,
    this.onTap,
  });

  final String label;
  final bool raised;
  final TantrumMetrics m;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onTap: onTap,
      child: SizedBox(
        width: 52 * m.scale,
        height: 20 * m.scale,
        child: CustomPaint(painter: _CapsulePainter(label, raised, m)),
      ),
    );
  }
}

class _CapsulePainter extends CustomPainter {
  const _CapsulePainter(this.label, this.raised, this.m);

  final String label;
  final bool raised;
  final TantrumMetrics m;

  @override
  void paint(Canvas canvas, Size size) {
    final s = m.scale;
    final shape = RRect.fromRectAndRadius(Offset.zero & size, Radius.circular(5 * s));
    if (raised) {
      paintRaise(canvas, shape, s, depth: 0.8);
      canvas.drawRRect(
        shape,
        Paint()
          ..shader = faceShader(Offset.zero & size, const [Colors.white, TantrumTokens.capBottom],
              light: const Alignment(-0.3, -1)),
      );
      paintBevel(canvas, shape, s, width: 0.9);
    } else {
      canvas.drawRRect(
        shape,
        Paint()
          ..shader = faceShader(Offset.zero & size, const [
            TantrumTokens.meterFloorDark,
            TantrumTokens.meterFloor,
          ], light: const Alignment(-0.3, -1)),
      );
      paintRecess(canvas, shape, s, depth: 1.0);
    }
    final tp = TextPainter(
      text: TextSpan(
        text: label,
        style: TantrumTokens.caption(s * 1.1,
            color: raised ? TantrumTokens.inkStrong : TantrumTokens.ink),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    tp.paint(canvas, Offset((size.width - tp.width) / 2, (size.height - tp.height) / 2));
  }

  @override
  bool shouldRepaint(_CapsulePainter old) =>
      old.label != label || old.raised != raised || old.m.scale != m.scale;
}

/// 上下箭头一对（图1 中间那一组）
class TantrumChevronPair extends StatelessWidget {
  const TantrumChevronPair({super.key, required this.m, this.onUp, this.onDown});

  final TantrumMetrics m;
  final VoidCallback? onUp;
  final VoidCallback? onDown;

  @override
  Widget build(BuildContext context) {
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        _chevron(Icons.keyboard_arrow_up_rounded, onUp),
        SizedBox(height: 3 * m.scale),
        _chevron(Icons.keyboard_arrow_down_rounded, onDown),
      ],
    );
  }

  Widget _chevron(IconData icon, VoidCallback? tap) {
    return GestureDetector(
      onTap: tap,
      child: Container(
        width: 26 * m.scale,
        height: 13 * m.scale,
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(4 * m.scale),
          gradient: LinearGradient(
            begin: const Alignment(-0.3, -1),
            end: const Alignment(0.3, 1),
            colors: [Colors.white, TantrumTokens.capBottom],
          ),
          boxShadow: [
            BoxShadow(
              color: TantrumTokens.rimLight,
              blurRadius: 2.2 * m.scale,
              offset: Offset(-1 * m.scale, -1.1 * m.scale),
            ),
            BoxShadow(
              color: TantrumTokens.castDark,
              blurRadius: 2.8 * m.scale,
              offset: Offset(1.2 * m.scale, 1.6 * m.scale),
            ),
          ],
        ),
        child: Icon(icon, size: 13 * m.scale, color: TantrumTokens.ink),
      ),
    );
  }
}

/// 竖直电平柱（图1 两侧的 IN / OUT）：沉槽 + 红色填充 + 帽 + 右侧刻度
class TantrumLevelMeter extends StatelessWidget {
  const TantrumLevelMeter({
    super.key,
    required this.m,
    required this.label,
    required this.level,
    required this.text,
  });

  final TantrumMetrics m;
  final String label;
  final double level;
  final String text;

  @override
  Widget build(BuildContext context) {
    // 柱身很窄，"-24.5" 这样的读数一旦折行就会把整条柱子顶出 bodyHeight：
    // 两处读数都锁成单行、宽度不够时缩放而不是换行。
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.center,
      children: [
        SizedBox(
          width: m.meterWidth,
          height: m.meterTravel,
          child: CustomPaint(painter: _MeterPainter(m, level)),
        ),
        SizedBox(height: 3 * m.scale),
        _MeterLabel(m: m, text: label),
        _MeterLabel(m: m, text: text, style: TantrumTokens.value(m.scale, color: TantrumTokens.accent)),
      ],
    );
  }
}

class _MeterLabel extends StatelessWidget {
  const _MeterLabel({required this.m, required this.text, this.style});

  final TantrumMetrics m;
  final String text;
  final TextStyle? style;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: m.meterWidth,
      child: FittedBox(
        fit: BoxFit.scaleDown,
        child: Text(
          text,
          maxLines: 1,
          style: style ?? TantrumTokens.caption(m.scale, color: TantrumTokens.ink),
        ),
      ),
    );
  }
}

class _MeterPainter extends CustomPainter {
  const _MeterPainter(this.m, this.level);

  final TantrumMetrics m;
  final double level;

  @override
  void paint(Canvas canvas, Size size) {
    final s = m.scale;
    final cx = size.width / 2;
    final chanW = m.meterSlotWidth;
    final channel = RRect.fromRectAndRadius(
      Rect.fromLTWH(cx - chanW / 2, 0, chanW, size.height),
      Radius.circular(chanW / 2),
    );
    // 槽底先压到比面板暗两档，再叠内阴影：只画一层浅灰会读成"贴了根白条"
    canvas.drawRRect(
      channel,
      Paint()
        ..shader = faceShader(channel.outerRect, const [
              TantrumTokens.meterFloorDark,
              Color(0xFFA9B2BF),
            ], light: const Alignment(-0.4, -1)),
    );
    paintRecess(canvas, channel, s, depth: 1.1);

    final top = size.height * (1 - level.clamp(0.0, 1.0));
    canvas.save();
    canvas.clipRRect(channel);
    final fill = Rect.fromLTRB(cx - chanW / 2, top, cx + chanW / 2, size.height);
    // 红色柱体本身带一点受光：中间亮、两侧暗，才像一根玻璃管里的光柱
    canvas.drawRect(
      fill,
      Paint()
        ..shader = LinearGradient(
          begin: Alignment.bottomCenter,
          end: Alignment.topCenter,
          colors: [
            TantrumTokens.accent.withValues(alpha: 0.85),
            TantrumTokens.accent,
          ],
        ).createShader(fill),
    );
    canvas.drawRect(
      fill,
      Paint()
        ..shader = LinearGradient(
          begin: Alignment.centerLeft,
          end: Alignment.centerRight,
          colors: [
            const Color(0x40000000),
            const Color(0x00000000),
            const Color(0x33FFFFFF),
          ],
          stops: const [0.0, 0.45, 1.0],
        ).createShader(fill),
    );
    // 顶端的一道亮沿 = 峰值
    canvas.drawLine(
      Offset(cx - chanW / 2, top + 0.8 * s),
      Offset(cx + chanW / 2, top + 0.8 * s),
      Paint()
        ..color = Colors.white.withValues(alpha: 0.75)
        ..strokeWidth = 1.4 * s,
    );
    canvas.restore();
    // 柱头溢出槽外的一层红雾：LED 光打在塑料上的感觉，范围要收得住，
    // 扩散到槽外就成了"糖葫芦"。
    canvas.save();
    canvas.clipRRect(
        RRect.fromRectAndRadius(
            Rect.fromLTWH(cx - chanW / 2 - 2 * s, top - chanW * 1.2, chanW + 4 * s, chanW * 2.4),
            Radius.circular(chanW * 0.5)));
    canvas.drawCircle(Offset(cx, top), chanW * 0.55,
        Paint()
          ..color = TantrumTokens.accentSoft
          ..maskFilter = MaskFilter.blur(BlurStyle.normal, chanW * 0.3));
    canvas.restore();

    // 游标：压在填充顶端的一条细凸标，只比槽宽一点点
    final capW = chanW * 1.28;
    final capH = 2.8 * s;
    final cap = RRect.fromRectAndRadius(
      Rect.fromLTWH(cx - capW / 2, top - capH / 2, capW, capH),
      Radius.circular(capH / 2),
    );
    paintRaise(canvas, cap, s, depth: 0.4);
    canvas.drawRRect(
      cap,
      Paint()
        ..shader = faceShader(cap.outerRect, const [Colors.white, TantrumTokens.capBottom],
            light: const Alignment(-0.2, -1)),
    );
    paintBevel(canvas, cap, s, width: 0.5);

    // 外侧刻度梯：每 1/5 一根，0 dB 那根加长。
    // 梯子的外沿必须留在 meterWidth 里，否则会画到屏幕的沉槽上。
    final tickX0 = cx + chanW / 2 + 1.2 * s;
    for (var i = 0; i <= 5; i++) {
      final y = size.height * i / 5;
      final len = i == 1 ? 3.6 * s : 2.4 * s;
      canvas.drawLine(
        Offset(tickX0, y),
        Offset(tickX0 + len, y),
        Paint()
          ..color = i == 1 ? TantrumTokens.ink : TantrumTokens.tickIdle
          ..strokeWidth = 1,
      );
    }
  }

  @override
  bool shouldRepaint(_MeterPainter old) => old.level != level || old.m.scale != m.scale;
}

/// 读数项：小标签 + 红色数值（图1 的 PHASE AMOUNT 21）
class TantrumReadout extends StatelessWidget {
  const TantrumReadout({
    super.key,
    required this.label,
    required this.value,
    required this.m,
  });

  final String label;
  final String value;
  final TantrumMetrics m;

  @override
  Widget build(BuildContext context) {
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(label, style: TantrumTokens.caption(m.scale)),
        SizedBox(width: 5 * m.scale),
        Text(value, style: TantrumTokens.readout(m.scale)),
      ],
    );
  }
}
