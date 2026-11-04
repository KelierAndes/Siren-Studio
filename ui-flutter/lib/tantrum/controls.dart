import 'dart:math' as math;

import 'package:flutter/material.dart';

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
    final ticks = m.tickCount.toInt();
    final active = (value * (ticks - 1)).round();

    for (var i = 0; i < ticks; i++) {
      final angle = _knobStart + i / (ticks - 1) * _knobSweep;
      final painted = i <= active;
      canvas.drawCircle(
        center + Offset(math.cos(angle), math.sin(angle)) * m.knobRingRadius,
        m.tickRadius * (painted ? 1.0 : 0.78),
        Paint()
          ..color = painted ? TantrumTokens.tickActive : TantrumTokens.tickIdle,
      );
    }

    canvas.drawCircle(
      center + Offset(0, 2.2 * m.scale),
      r,
      Paint()
        ..color = TantrumTokens.knobShadow
        ..maskFilter = MaskFilter.blur(BlurStyle.normal, 3.4 * m.scale),
    );
    final faceRect = Rect.fromCircle(center: center, radius: r);
    canvas.drawCircle(
      center,
      r,
      Paint()
        ..shader = const LinearGradient(
          begin: Alignment.topCenter,
          end: Alignment.bottomCenter,
          colors: [TantrumTokens.knobTop, TantrumTokens.knobBottom],
        ).createShader(faceRect),
    );
    canvas.drawCircle(
      center,
      r,
      Paint()
        ..color = TantrumTokens.knobEdge
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1 * m.scale,
    );
    canvas.drawArc(
      Rect.fromCircle(center: center, radius: r - 0.9 * m.scale),
      math.pi * 1.1,
      math.pi * 0.8,
      false,
      Paint()
        ..color = const Color(0x99FFFFFF)
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.4 * m.scale
        ..strokeCap = StrokeCap.round,
    );

    final angle = _knobStart + value * _knobSweep;
    final mid = center + Offset(math.cos(angle), math.sin(angle)) * m.knobNotchRadius;
    canvas.save();
    canvas.translate(mid.dx, mid.dy);
    canvas.rotate(angle + math.pi / 2);
    canvas.drawRRect(
      RRect.fromRectAndRadius(
        Rect.fromCenter(center: Offset.zero, width: m.knobNotchWidth, height: m.knobNotchLength),
        Radius.circular(m.knobNotchWidth / 2),
      ),
      Paint()..color = TantrumTokens.notch,
    );
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
    canvas.drawRRect(
      RRect.fromRectAndRadius(
        Rect.fromLTWH(cx - m.faderSlotWidth / 2, top, m.faderSlotWidth, bottom - top),
        Radius.circular(m.faderSlotWidth / 2),
      ),
      Paint()..color = TantrumTokens.slot,
    );
    final capCenter = bottom - (bottom - top) * value;
    final cap = Rect.fromCenter(
      center: Offset(cx, capCenter),
      width: m.faderCapWidth,
      height: m.faderCapHeight,
    );
    canvas.drawRRect(
      RRect.fromRectAndRadius(cap.shift(Offset(0, 2 * m.scale)), Radius.circular(4 * m.scale)),
      Paint()
        ..color = TantrumTokens.knobShadow
        ..maskFilter = MaskFilter.blur(BlurStyle.normal, 2.2 * m.scale),
    );
    canvas.drawRRect(
      RRect.fromRectAndRadius(cap, Radius.circular(4 * m.scale)),
      Paint()
        ..shader = const LinearGradient(
          begin: Alignment.topCenter,
          end: Alignment.bottomCenter,
          colors: [TantrumTokens.capTop, TantrumTokens.capBottom],
        ).createShader(cap),
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
    canvas.drawRRect(
      RRect.fromRectAndRadius(track, Radius.circular(size.height / 2)),
      Paint()..color = TantrumTokens.segTrack,
    );
    final w = track.width / items.length;
    final active = Rect.fromLTWH(
      track.left + index * w + 1.5 * m.scale,
      1.5 * m.scale,
      w - 3 * m.scale,
      size.height - 3 * m.scale,
    );
    canvas.drawRRect(
      RRect.fromRectAndRadius(active, Radius.circular(active.height / 2)),
      Paint()
        ..color = TantrumTokens.knobShadow
        ..maskFilter = MaskFilter.blur(BlurStyle.normal, 1.6 * m.scale),
    );
    canvas.drawRRect(
      RRect.fromRectAndRadius(active, Radius.circular(active.height / 2)),
      Paint()..color = TantrumTokens.segOn,
    );
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
    canvas.drawRRect(
      RRect.fromRectAndRadius(Offset.zero & size, Radius.circular(size.height / 2)),
      Paint()..color = on ? TantrumTokens.switchOn : TantrumTokens.switchOff,
    );
    final r = size.height / 2 - 2 * m.scale;
    final center = Offset(on ? size.width - r - 2 * m.scale : r + 2 * m.scale, size.height / 2);
    canvas.drawCircle(
      center + Offset(0, 1.4 * m.scale),
      r,
      Paint()
        ..color = TantrumTokens.panelShadow
        ..maskFilter = MaskFilter.blur(BlurStyle.normal, 2 * m.scale),
    );
    canvas.drawCircle(center, r, Paint()..color = Colors.white);
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
    if (!onDark) {
      canvas.drawCircle(
        center + Offset(0, 1.2 * m.scale),
        r,
        Paint()
          ..color = TantrumTokens.knobShadow
          ..maskFilter = MaskFilter.blur(BlurStyle.normal, 2 * m.scale),
      );
      canvas.drawCircle(center, r, Paint()..color = Colors.white);
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
      child: CustomPaint(painter: _LedPainter(on)),
    );
  }
}

class _LedPainter extends CustomPainter {
  const _LedPainter(this.on);

  final bool on;

  @override
  void paint(Canvas canvas, Size size) {
    final center = size.center(Offset.zero);
    canvas.drawCircle(center, size.width / 2, Paint()..color = TantrumTokens.ledOff);
    canvas.drawCircle(
      center,
      size.width / 2 - 1.2,
      Paint()..color = on ? TantrumTokens.ledOn : TantrumTokens.ledOff,
    );
    if (on) {
      canvas.drawCircle(
        center,
        size.width / 2,
        Paint()
          ..color = TantrumTokens.accentSoft
          ..maskFilter = MaskFilter.blur(BlurStyle.normal, 3),
      );
    }
  }

  @override
  bool shouldRepaint(_LedPainter old) => old.on != on;
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
          color: Colors.white,
          borderRadius: BorderRadius.circular(4 * m.scale),
          boxShadow: [
            BoxShadow(
              color: TantrumTokens.knobShadow,
              blurRadius: 2 * m.scale,
              offset: Offset(0, 1 * m.scale),
            ),
          ],
        ),
        child: Icon(icon, size: 13 * m.scale, color: TantrumTokens.ink),
      ),
    );
  }
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
