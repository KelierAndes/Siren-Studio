import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'theme.dart';

/// 双击复位的判定窗口与容差（旋钮、滑条共用）
const Duration _doubleTapWindow = Duration(milliseconds: 500);
const double _doubleTapRadius = 14;

/// 控件的物理量域：控件一律收发绝对值，归一化只在内部发生。
class NeuRange {
  const NeuRange(this.min, this.max, {this.defaultValue = 0});

  final double min;
  final double max;
  final double defaultValue;

  double clampAbs(double v) => v.clamp(min, max);
  double norm(double v) => ((v - min) / (max - min)).clamp(0.0, 1.0);
  double abs(double t) => min + (max - min) * t.clamp(0.0, 1.0);
}

/// 金属旋钮：深色圆顶 + 彩色指针 + 12 点定位刻线。
class NeuKnob extends StatefulWidget {
  const NeuKnob({
    super.key,
    required this.palette,
    required this.value,
    required this.range,
    required this.onChanged,
    this.size = 108,
    this.accentColor,
    this.start = -140,
    this.sweep = 280,
  });

  final MetalPalette palette;
  final double value;
  final NeuRange range;
  final ValueChanged<double> onChanged;
  final double size;
  final Color? accentColor;
  final double start;
  final double sweep;

  @override
  State<NeuKnob> createState() => _NeuKnobState();
}

class _NeuKnobState extends State<NeuKnob> {
  bool _dragging = false;
  DateTime? _lastTapUp;
  Offset _lastTapPos = Offset.zero;

  static const double _travelPx = 260;

  /// 滚轮语义，按按下位置二选一，整段手势不再切换模型：
  /// * 按在外圈（半径 42% 以外）→ **绝对角度跟踪**，指示线正对鼠标，偏差恒为 0。
  /// * 按在中心区 → 竖向位移模型（小旋钮瞄不准，也用它做精调）。
  static const double _rotaryDeadFraction = .42;

  /// 绝对模式下指针离圆心小于此值时角度不可信
  static const double _minAngleRadius = 10;

  bool? _useRotary;
  Offset _prevLocal = Offset.zero;

  Offset get _center => Offset(widget.size / 2, widget.size / 2);

  void _apply(double normDelta) {
    final t = (widget.range.norm(widget.value) + normDelta).clamp(0.0, 1.0);
    widget.onChanged(widget.range.abs(t));
  }

  void _onDown(DragDownDetails d) {
    _prevLocal = d.localPosition;
    // 按在外圈 → 绝对角度跟踪；按在中心区 → 竖向位移。整段手势不再切换。
    _useRotary = (d.localPosition - _center).distance >= widget.size / 2 * _rotaryDeadFraction;
    if (_useRotary!) _steerAbsolute(d.localPosition);
  }

  void _onDragStart(DragStartDetails d) {
    setState(() => _dragging = true);
    if (_useRotary ?? false) {
      _steerAbsolute(d.localPosition);
    } else {
      _prevLocal = d.localPosition;
    }
  }

  void _onDrag(DragUpdateDetails d) {
    if (_useRotary ?? false) {
      _steerAbsolute(d.localPosition);
    } else {
      final v = d.localPosition - _prevLocal;
      _prevLocal = d.localPosition;
      _apply(-v.dy / _travelPx * (_fine ? .25 : 1));
    }
  }

  /// 指示线正对鼠标：偏差恒为 0。指针滑进圆心附近时角度无意义，保持不动。
  void _steerAbsolute(Offset local) {
    final r = local - _center;
    if (r.distance < _minAngleRadius) return;
    final target = widget.range.abs(_fractionAt(local));
    if (!_fine) {
      widget.onChanged(target);
      return;
    }
    // 按住 Shift/Ctrl：只向目标角度靠一小步，用于精调
    widget.onChanged(widget.range.clampAbs(widget.value + (target - widget.value) * .12));
  }

  /// 指针相对圆心的角度（0 = 12 点，顺时针为正）换算成量程比例。
  /// start=-140 / sweep=280 时，左下缺口自动夹到 0、右下缺口夹到 1。
  double _fractionAt(Offset local) {
    final r = local - _center;
    final deg = math.atan2(r.dx, -r.dy) * 180 / math.pi;
    return ((deg - widget.start) / widget.sweep).clamp(0.0, 1.0);
  }

  @override
  Widget build(BuildContext context) {
    return Listener(
      onPointerSignal: (e) {
        if (e is! PointerScrollEvent) return;
        _apply(-e.scrollDelta.dy.sign * .02);
      },
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onPanDown: _onDown,
        onPanStart: _onDragStart,
        onPanUpdate: _onDrag,
        onPanEnd: (_) => setState(() => _dragging = false),
        onPanCancel: () => setState(() => _dragging = false),
        onTapUp: _tapUp,
        child: CustomPaint(
          size: Size(widget.size, widget.size),
          painter: _KnobPainter(
            palette: widget.palette,
            t: widget.range.norm(widget.value),
            accent: widget.accentColor ?? widget.palette.cyan,
            dragging: _dragging,
            start: widget.start,
            sweep: widget.sweep,
            ),
        ),
      ),
    );
  }

  bool get _fine => HardwareKeyboard.instance.isShiftPressed || HardwareKeyboard.instance.isControlPressed;

  /// 双击复位，见 [_doubleTapRadius]：必须同处且 500ms 内。
  void _tapUp(TapUpDetails d) {
    final prev = _lastTapUp;
    final sameSpot = (d.localPosition - _lastTapPos).distance <= _doubleTapRadius;
    final now = DateTime.now();
    if (prev != null && now.difference(prev) < _doubleTapWindow && sameSpot) {
      _lastTapUp = null;
      widget.onChanged(widget.range.defaultValue);
    } else {
      _lastTapUp = now;
      _lastTapPos = d.localPosition;
    }
  }
}

class _KnobPainter extends CustomPainter {
  _KnobPainter({
    required this.palette,
    required this.t,
    required this.accent,
    required this.dragging,
    required this.start,
    required this.sweep,
  });

  final MetalPalette palette;
  final double t;
  final Color accent;
  final bool dragging;
  final double start;
  final double sweep;

  @override
  void paint(Canvas canvas, Size size) {
    final c = size.center(Offset.zero);
    final r = size.shortestSide / 2;
    final dome = r - 6;

    canvas.drawCircle(
      c + const Offset(0, 3),
      dome,
      Paint()
        ..color = palette.shade(.55)
        ..maskFilter = const MaskFilter.blur(BlurStyle.normal, 5),
    );
    canvas.drawCircle(
      c,
      r - 1.5,
      Paint()
        ..shader = ui.Gradient.linear(
          Offset(c.dx, c.dy - r),
          Offset(c.dx, c.dy + r),
          [palette.bevel, palette.knobRing],
        ),
    );
    canvas.drawCircle(c, r - 3.5, Paint()..color = palette.knobNeck);
    canvas.drawCircle(
      c,
      dome,
      Paint()
        ..shader = ui.Gradient.radial(
          c - Offset(dome * .45, dome * .55),
          dome * 2.1,
          [palette.knobTop, palette.knobMid, palette.knobBottom],
          [.02, .5, 1],
        ),
    );
    canvas.drawArc(
      Rect.fromCircle(center: c, radius: dome - .6),
      math.pi * 1.05,
      math.pi * .9,
      false,
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.2
        ..shader = ui.Gradient.linear(
          Offset(c.dx - dome, c.dy),
          Offset(c.dx + dome, c.dy),
          [Colors.transparent, palette.glint(.34), Colors.transparent],
          [0, .5, 1],
        ),
    );
    canvas.drawCircle(
      c,
      dome - .6,
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1
        ..color = palette.shade(.35),
    );

    canvas.drawLine(
      Offset(c.dx, c.dy - r + 1),
      Offset(c.dx, c.dy - r + 5.5),
      Paint()
        ..strokeWidth = 1.4
        ..strokeCap = StrokeCap.round
        ..color = palette.ink,
    );

    final ang = (start + sweep * t - 90) * math.pi / 180;
    final dir = Offset(math.cos(ang), math.sin(ang));
    final p0 = c + dir * (dome * .34);
    final p1 = c + dir * (dome * .88);
    canvas.drawLine(
      p0 + const Offset(0, 1),
      p1 + const Offset(0, 1),
      Paint()
        ..strokeWidth = 3.4
        ..strokeCap = StrokeCap.round
        ..color = palette.shade(.55),
    );
    canvas.drawLine(
      p0,
      p1,
      Paint()
        ..strokeWidth = 2.8
        ..strokeCap = StrokeCap.round
        ..color = accent.withValues(alpha: dragging ? 1 : .92)
        ..maskFilter = MaskFilter.blur(BlurStyle.normal, dragging ? 2.5 : .6),
    );
  }

  @override
  bool shouldRepaint(_KnobPainter old) =>
      old.t != t || old.dragging != dragging || old.accent != accent || old.palette != palette;
}

/// 水平滑槽：内凹导轨 + 金属帽
class NeuSlider extends StatefulWidget {
  const NeuSlider({
    super.key,
    required this.palette,
    required this.value,
    required this.range,
    required this.onChanged,
    required this.width,
    this.height = 22,
    this.accentColor,
    this.centerOrigin = false,
  });

  final MetalPalette palette;
  final double value;
  final NeuRange range;
  final ValueChanged<double> onChanged;
  final double width;
  final double height;
  final Color? accentColor;
  final bool centerOrigin;

  @override
  State<NeuSlider> createState() => _NeuSliderState();
}

class _NeuSliderState extends State<NeuSlider> {
  bool _dragging = false;
  DateTime? _lastTapUp;
  Offset _lastTapPos = Offset.zero;
  static const double _capW = 20;

  /// 滑块中心的可达区间是 [capW/2, width - capW/2]，与绘制侧共用同一个量
  double get _travel => widget.width - _capW;

  double _valueAtX(double dx) => widget.range.abs(((dx - _capW / 2) / _travel).clamp(0.0, 1.0));

  bool get _fine =>
      HardwareKeyboard.instance.isShiftPressed || HardwareKeyboard.instance.isControlPressed;

  /// 双击复位。要求两次点击落在同一处（14px 内）且间隔小于 500ms：
  /// 少了位置这条，"快速点两个不同位置"会被误判成复位，把用户要去的位置吃掉。
  void _tapUp(TapUpDetails d) {
    final prev = _lastTapUp;
    final sameSpot = (d.localPosition - _lastTapPos).distance <= _doubleTapRadius;
    final now = DateTime.now();
    if (prev != null && now.difference(prev) < _doubleTapWindow && sameSpot) {
      _lastTapUp = null;
      widget.onChanged(widget.range.defaultValue);
    } else {
      _lastTapUp = now;
      _lastTapPos = d.localPosition;
    }
  }

  @override
  Widget build(BuildContext context) {
    final accent = widget.accentColor ?? widget.palette.cyan;
    return Listener(
      onPointerSignal: (e) {
        if (e is! PointerScrollEvent) return;
        widget.onChanged(widget.range.abs((widget.range.norm(widget.value) - e.scrollDelta.dy.sign * .02).clamp(0.0, 1.0)));
      },
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        // 绝对跟踪：滑块始终在指针底下，在端点被夹住也不会留下永久错位。
        // 用 onPan* 而非 onHorizontalDrag*：后者在拖动变斜时会被轴向否决而中途停住。
        onPanDown: (d) {
          if (!_fine) widget.onChanged(_valueAtX(d.localPosition.dx));
        },
        onPanStart: (d) {
          setState(() => _dragging = true);
          if (!_fine) widget.onChanged(_valueAtX(d.localPosition.dx));
        },
        onPanUpdate: (d) {
          if (_fine) {
            final t = (widget.range.norm(widget.value) + d.delta.dx * .25 / _travel).clamp(0.0, 1.0);
            widget.onChanged(widget.range.abs(t));
          } else {
            widget.onChanged(_valueAtX(d.localPosition.dx));
          }
        },
        onPanEnd: (_) => setState(() => _dragging = false),
        onPanCancel: () => setState(() => _dragging = false),
        onTapUp: _tapUp,
        child: CustomPaint(
          size: Size(widget.width, widget.height),
          painter: _SliderPainter(
            palette: widget.palette,
            t: widget.range.norm(widget.value),
            accent: accent,
            dragging: _dragging,
            centerOrigin: widget.centerOrigin,
          ),
        ),
      ),
    );
  }
}

class _SliderPainter extends CustomPainter {
  _SliderPainter({
    required this.palette,
    required this.t,
    required this.accent,
    required this.dragging,
    required this.centerOrigin,
  });

  final MetalPalette palette;
  final double t;
  final Color accent;
  final bool dragging;
  final bool centerOrigin;

  @override
  void paint(Canvas canvas, Size size) {
    const capW = 20.0;
    final slot = RRect.fromRectAndRadius(
      Rect.fromLTWH(0, size.height / 2 - 4, size.width, 8),
      const Radius.circular(4),
    );
    canvas.drawRRect(slot, Paint()..color = palette.slotFloor);
    canvas.save();
    canvas.clipRRect(slot);
    canvas.drawRRect(
      slot.shift(const Offset(0, 1.6)),
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 5
        ..color = palette.shade(.9)
        ..maskFilter = const MaskFilter.blur(BlurStyle.normal, 2.5),
    );
    final travel = size.width - capW;
    final x = capW / 2 + travel * t;
    if (centerOrigin) {
      final mid = size.width / 2;
      canvas.drawRect(
        Rect.fromLTWH(math.min(mid, x), size.height / 2 - 4, (x - mid).abs(), 8),
        Paint()..color = accent.withValues(alpha: .55),
      );
      canvas.drawLine(
        Offset(mid, size.height / 2 - 5),
        Offset(mid, size.height / 2 + 5),
        Paint()
          ..strokeWidth = 1.2
          ..color = palette.inkFaint,
      );
    } else {
      canvas.drawRect(
        Rect.fromLTWH(2, size.height / 2 - 4, (x - 2).clamp(0.0, size.width - 4), 8),
        Paint()..color = accent.withValues(alpha: .45),
      );
    }
    canvas.restore();

    final cap = Rect.fromLTWH(x - capW / 2, 1, capW, size.height - 2);
    canvas.drawRRect(
      RRect.fromRectAndRadius(cap, const Radius.circular(6)),
      Paint()
        ..color = palette.shade(dragging ? .8 : .6)
        ..maskFilter = MaskFilter.blur(BlurStyle.normal, dragging ? 5 : 3),
    );
    canvas.drawRRect(
      RRect.fromRectAndRadius(cap, const Radius.circular(6)),
      Paint()
        ..shader = ui.Gradient.linear(
          Offset(cap.left, cap.top),
          Offset(cap.left, cap.bottom),
          [palette.knobTop, palette.knobMid, palette.knobBottom],
          [0, .5, 1],
        ),
    );
    canvas.drawLine(
      Offset(x, cap.top + 5),
      Offset(x, cap.bottom - 5),
      Paint()
        ..strokeWidth = 1.6
        ..strokeCap = StrokeCap.round
        ..color = accent,
    );
  }

  @override
  bool shouldRepaint(_SliderPainter old) =>
      old.t != t || old.dragging != dragging || old.centerOrigin != centerOrigin || old.palette != palette;
}

/// LCD 读数屏（参考图的 FREQ / WIDTH 小窗）
class NeuLcd extends StatelessWidget {
  const NeuLcd({
    super.key,
    required this.palette,
    required this.value,
    this.unit,
    this.topLabels,
    this.accent,
    this.width = 92,
    this.height = 44,
    this.fontSize = 21,
  });

  final MetalPalette palette;
  final String value;
  final String? unit;
  final List<String>? topLabels;
  final Color? accent;
  final double width;
  final double height;
  final double fontSize;

  @override
  Widget build(BuildContext context) {
    final c = accent ?? palette.cyan;
    return NeuWell(
      size: Size(width, height),
      radius: 7,
      palette: palette,
      tint: palette.lcd,
      padding: const EdgeInsets.fromLTRB(7, 5, 7, 0),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (topLabels != null)
            Row(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              children: [
                for (final l in topLabels!)
                  Expanded(
                    child: FittedBox(
                      fit: BoxFit.scaleDown,
                      alignment: Alignment.centerLeft,
                      child: Text(l, style: NeuType.chrome(7.5, palette.lcdInk, tracking: .6)),
                    ),
                  ),
              ],
            )
          else
            const SizedBox(height: 8),
          FittedBox(
            fit: BoxFit.scaleDown,
            alignment: Alignment.centerLeft,
            child: Row(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.baseline,
              textBaseline: TextBaseline.alphabetic,
              children: [
                Text(
                  value,
                  style: NeuType.numeric(fontSize, c).copyWith(
                    shadows: [Shadow(color: c.withValues(alpha: .5), blurRadius: 8)],
                  ),
                ),
                if (unit != null) Text(' $unit', style: NeuType.chrome(10, c.withValues(alpha: .8), tracking: .5)),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// 可直接输入的数值牌：点一下就地变输入框
class NeuValueField extends StatefulWidget {
  const NeuValueField({
    super.key,
    required this.palette,
    required this.value,
    required this.range,
    required this.onChanged,
    required this.width,
    this.height = 26,
    this.unit,
    this.accent,
    this.digits = 1,
    this.fontSize = 12.5,
    this.format,
  });

  final MetalPalette palette;
  final double value;
  final NeuRange range;
  final ValueChanged<double> onChanged;
  final double width;
  final double height;
  final String? unit;
  final Color? accent;
  final int digits;
  final double fontSize;
  final String Function(double)? format;

  @override
  State<NeuValueField> createState() => _NeuValueFieldState();
}

class _NeuValueFieldState extends State<NeuValueField> {
  bool _editing = false;
  final TextEditingController _ctl = TextEditingController();
  late final FocusNode _focus = FocusNode()..addListener(_onFocus);

  void _onFocus() {
    if (!_focus.hasFocus && _editing) _commit(_ctl.text);
  }

  void _begin() {
    _ctl.text = widget.value.toStringAsFixed(widget.digits);
    setState(() => _editing = true);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  void _commit(String text) {
    final v = double.tryParse(text.trim());
    if (v != null) widget.onChanged(widget.range.clampAbs(v));
    if (mounted) setState(() => _editing = false);
  }

  @override
  void dispose() {
    _focus.removeListener(_onFocus);
    _focus.dispose();
    _ctl.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final c = widget.accent ?? widget.palette.cyan;
    if (_editing) {
      return NeuWell(
        size: Size(widget.width, widget.height),
        radius: 6,
        palette: widget.palette,
        tint: widget.palette.editBg,
        child: Row(
          children: [
            Expanded(
              child: Padding(
                padding: const EdgeInsets.only(left: 6),
                child: TextField(
                  controller: _ctl,
                  focusNode: _focus,
                  keyboardType: const TextInputType.numberWithOptions(decimal: true, signed: true),
                  textAlign: TextAlign.right,
                  cursorColor: c,
                  style: NeuType.numeric(widget.fontSize, c),
                  decoration: const InputDecoration(
                    isDense: true,
                    contentPadding: EdgeInsets.zero,
                    border: InputBorder.none,
                    enabledBorder: InputBorder.none,
                    focusedBorder: InputBorder.none,
                  ),
                  onSubmitted: _commit,
                ),
              ),
            ),
            if (widget.unit != null)
              Padding(
                padding: const EdgeInsets.only(right: 6),
                child: Text(widget.unit!, style: NeuType.chrome(9, c.withValues(alpha: .75), tracking: .4)),
              ),
          ],
        ),
      );
    }
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: _begin,
      child: Tooltip(
        message: '点击直接输入',
        child: NeuWell(
          size: Size(widget.width, widget.height),
          radius: 6,
          palette: widget.palette,
          tint: widget.palette.lcd,
          child: Row(
            mainAxisAlignment: MainAxisAlignment.center,
            crossAxisAlignment: CrossAxisAlignment.baseline,
            textBaseline: TextBaseline.alphabetic,
            children: [
              Flexible(
                child: Text(
                  _shown(),
                  maxLines: 1,
                  overflow: TextOverflow.clip,
                  style: NeuType.numeric(widget.fontSize, c),
                ),
              ),
              if (widget.unit != null)
                Text(' ${widget.unit!}', style: NeuType.chrome(8.5, c.withValues(alpha: .7), tracking: .3)),
            ],
          ),
        ),
      ),
    );
  }

  String _shown() {
    if (widget.format != null) return widget.format!(widget.value);
    final v = widget.value;
    return v >= 0 && widget.range.min < 0 ? '+${v.toStringAsFixed(widget.digits)}' : v.toStringAsFixed(widget.digits);
  }
}

/// 金属胶囊按钮
class NeuPill extends StatefulWidget {
  const NeuPill({
    super.key,
    required this.palette,
    required this.label,
    required this.onTap,
    this.active = false,
    this.accent,
    this.led = false,
    this.height = 30,
    this.fontSize = 12,
    this.fill = false,
  });

  final MetalPalette palette;
  final String label;
  final VoidCallback onTap;
  final bool active;
  final Color? accent;
  final bool led;
  final double height;
  final double fontSize;
  final bool fill;

  @override
  State<NeuPill> createState() => _NeuPillState();
}

class _NeuPillState extends State<NeuPill> {
  bool _down = false;

  @override
  Widget build(BuildContext context) {
    final accent = widget.accent ?? widget.palette.cyan;
    final on = widget.active;
    final pill = GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTapDown: (_) => setState(() => _down = true),
      onTapCancel: () => setState(() => _down = false),
      onTapUp: (_) => setState(() => _down = false),
      onTap: widget.onTap,
      child: AnimatedContainer(
        duration: const Duration(milliseconds: 110),
        height: widget.height,
        padding: EdgeInsets.symmetric(horizontal: on ? 12 : 11),
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(widget.height / 2),
          gradient: LinearGradient(
            begin: Alignment.topCenter,
            end: Alignment.bottomCenter,
            colors: on
                ? [widget.palette.moduleTop, widget.palette.moduleBottom]
                : [widget.palette.pillOffTop, widget.palette.pillOffBottom],
          ),
          boxShadow: [
            if (on)
              BoxShadow(
                color: widget.palette.shade(_down ? .4 : .6),
                offset: Offset(0, _down ? 1 : 3),
                blurRadius: _down ? 3 : 7,
              )
            else
              BoxShadow(color: widget.palette.bevel.withValues(alpha: .22), offset: const Offset(0, -1)),
            if (on)
              BoxShadow(
                color: accent.withValues(alpha: .35),
                offset: Offset.zero,
                blurRadius: 10,
                spreadRadius: -2,
              ),
          ],
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            if (widget.led) ...[
              NeuLed(palette: widget.palette, color: on ? accent : widget.palette.ledDim, on: on, size: 7),
              const SizedBox(width: 6),
            ],
            Flexible(
              child: FittedBox(
                fit: BoxFit.scaleDown,
                child: Text(
                  widget.label,
                  style: NeuType.chrome(
                    widget.fontSize,
                    on ? widget.palette.inkStrong : widget.palette.inkFaint,
                    w: on ? FontWeight.w700 : FontWeight.w600,
                    tracking: 1.1,
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
    return widget.fill ? SizedBox(width: double.infinity, child: Center(child: pill)) : pill;
  }
}

class NeuLed extends StatelessWidget {
  const NeuLed({super.key, required this.palette, required this.color, required this.on, this.size = 8});
  final MetalPalette palette;
  final Color color;
  final bool on;
  final double size;

  @override
  Widget build(BuildContext context) {
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        shape: BoxShape.circle,
        color: on ? color : palette.ledOff,
        boxShadow: [
          if (on)
            BoxShadow(color: color.withValues(alpha: .9), blurRadius: 8, spreadRadius: 1)
          else
            BoxShadow(color: palette.shade(.54), offset: const Offset(0, 1), blurRadius: 1),
        ],
      ),
    );
  }
}

/// 模块标题右侧的小圆钮
class TinyRoundButton extends StatelessWidget {
  const TinyRoundButton({
    super.key,
    required this.palette,
    required this.on,
    required this.onTap,
    this.color,
    this.size = 15,
  });

  final MetalPalette palette;
  final bool on;
  final VoidCallback onTap;
  final Color? color;
  final double size;

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: onTap,
      child: SizedBox(
        width: size,
        height: size,
        child: CustomPaint(painter: _TinyBtnPainter(palette: palette, on: on, color: color ?? palette.ledOn)),
      ),
    );
  }
}

class _TinyBtnPainter extends CustomPainter {
  _TinyBtnPainter({required this.palette, required this.on, required this.color});
  final MetalPalette palette;
  final bool on;
  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    final c = size.center(Offset.zero);
    final r = size.shortestSide / 2;
    canvas.drawCircle(
      c + const Offset(0, 1),
      r,
      Paint()
        ..color = palette.shade(.6)
        ..maskFilter = const MaskFilter.blur(BlurStyle.normal, 1.6),
    );
    canvas.drawCircle(
      c,
      r - .5,
      Paint()
        ..shader = ui.Gradient.radial(
          c - Offset(r * .4, r * .5),
          r * 2,
          [palette.knobTop, palette.knobBottom],
        ),
    );
    canvas.drawCircle(
      c,
      r - .5,
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1
        ..color = on ? color.withValues(alpha: .9) : palette.knobRing,
    );
    if (on) {
      canvas.drawCircle(
        c,
        r * .34,
        Paint()
          ..color = color
          ..maskFilter = const MaskFilter.blur(BlurStyle.normal, 2),
      );
    }
  }

  @override
  bool shouldRepaint(_TinyBtnPainter old) => old.on != on || old.color != color || old.palette != palette;
}

/// 发光指示窗（TONE 的 TUNE 橙灯）
class NeuLamp extends StatelessWidget {
  const NeuLamp({super.key, required this.palette, required this.color, required this.size, this.label = 'TUNE'});
  final MetalPalette palette;
  final Color color;
  final Size size;
  final String label;

  @override
  Widget build(BuildContext context) {
    return NeuWell(
      size: size,
      radius: 8,
      palette: palette,
      tint: palette.lcd,
      child: Stack(
        fit: StackFit.expand,
        children: [
          CustomPaint(painter: _LampPainter(palette: palette, color: color)),
          Center(
            child: Text(
              label,
              style: NeuType.chrome(8.5, palette.screenInk(.85), tracking: 1.6, w: FontWeight.w700),
            ),
          ),
        ],
      ),
    );
  }
}

class _LampPainter extends CustomPainter {
  _LampPainter({required this.palette, required this.color});
  final MetalPalette palette;
  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    final c = Offset(size.width * .5, size.height * .58);
    canvas.drawCircle(
      c,
      size.width * .3,
      Paint()
        ..color = color
        ..maskFilter = MaskFilter.blur(BlurStyle.normal, size.width * .22),
    );
    canvas.drawCircle(
      c,
      size.width * .12,
      Paint()
        ..color = palette.nodeCore.withValues(alpha: .85)
        ..maskFilter = const MaskFilter.blur(BlurStyle.normal, 3),
    );
  }

  @override
  bool shouldRepaint(_LampPainter old) => old.color != color || old.palette != palette;
}

/// 垂直推子（IN / OUT）
class NeuFader extends StatelessWidget {
  const NeuFader({
    super.key,
    required this.palette,
    required this.value,
    required this.range,
    required this.onChanged,
    required this.height,
    this.width = 44,
    this.accent,
    this.ticks = 6,
  });

  final MetalPalette palette;
  final double value;
  final NeuRange range;
  final ValueChanged<double> onChanged;
  final double height;
  final double width;
  final Color? accent;
  final int ticks;

  @override
  Widget build(BuildContext context) {
    final a = accent ?? palette.cyan;
    return NeuWell(
      size: Size(width, height),
      radius: 9,
      palette: palette,
      deep: true,
      tint: palette.faderWell,
      child: LayoutBuilder(
        builder: (context, box) {
          final h = box.maxHeight;
          final geom = FaderGeom(h);
          // 绝对跟踪 + onPan*：帽子中心始终在指针底下，斜着拖也不会被轴向否决
          void setAt(double dy) => onChanged(range.abs(geom.tAt(dy)));
          return Listener(
            onPointerSignal: (e) {
              if (e is! PointerScrollEvent) return;
              onChanged(range.abs((range.norm(value) - e.scrollDelta.dy.sign * .02).clamp(0.0, 1.0)));
            },
            child: GestureDetector(
              behavior: HitTestBehavior.opaque,
              onPanDown: (d) => setAt(d.localPosition.dy),
              onPanUpdate: (d) {
                final fine = HardwareKeyboard.instance.isShiftPressed ||
                    HardwareKeyboard.instance.isControlPressed;
                if (fine) {
                  onChanged(range.abs((range.norm(value) - d.delta.dy * .25 / geom.stroke).clamp(0.0, 1.0)));
                } else {
                  setAt(d.localPosition.dy);
                }
              },
              child: CustomPaint(
                size: Size(width, h),
                painter: _FaderPainter(
                  palette: palette,
                  t: range.norm(value),
                  accent: a,
                  ticks: ticks,
                  geom: geom,
                ),
              ),
            ),
          );
        },
      ),
    );
  }
}

/// 推子几何：映射与绘制必须共用同一套数字，
/// 否则帽子会按 (h-2p-c)/(h-2p) 的比例慢于指针（原来就是这个不一致导致不跟手），
/// 短推子（手机版 52px）甚至会算出负的行程。
class FaderGeom {
  FaderGeom(this.h)
      : capH = (h * .13).clamp(16.0, 32.0),
        pad = (h * .06).clamp(6.0, 17.0);

  final double h;
  final double capH;
  final double pad;

  /// 帽子中心的行程
  double get stroke => math.max(1.0, h - pad * 2 - capH);

  double capYAt(double t) => pad + (1 - t) * stroke;

  double tAt(double dy) => (1 - (dy - (pad + capH / 2)) / stroke).clamp(0.0, 1.0);
}

class _FaderPainter extends CustomPainter {
  _FaderPainter({
    required this.palette,
    required this.t,
    required this.accent,
    required this.ticks,
    required this.geom,
  });
  final MetalPalette palette;
  final double t;
  final Color accent;
  final int ticks;
  final FaderGeom geom;

  @override
  void paint(Canvas canvas, Size size) {
    final cx = size.width / 2;
    final capH = geom.capH;
    final pad = geom.pad;
    final strokeH = geom.stroke;

    final slot = RRect.fromRectAndRadius(
      Rect.fromLTWH(cx - 4, pad, 8, size.height - pad * 2),
      const Radius.circular(4),
    );
    canvas.drawRRect(slot, Paint()..color = palette.faderSlot);
    canvas.save();
    canvas.clipRRect(slot);
    canvas.drawRRect(
      slot.shift(const Offset(1.4, 0)),
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 5
        ..color = palette.shade(.95)
        ..maskFilter = const MaskFilter.blur(BlurStyle.normal, 2.5),
    );
    final capY = pad + (1 - t) * strokeH;
    canvas.drawRect(
      Rect.fromLTWH(cx - 4, capY + capH / 2, 8, size.height - pad - (capY + capH / 2)),
      Paint()..color = accent.withValues(alpha: .45),
    );
    canvas.restore();

    for (var i = 0; i <= ticks; i++) {
      final y = pad + strokeH * (i / ticks) + capH / 2;
      final w = i % 2 == 0 ? 6.0 : 3.5;
      final p = Paint()
        ..strokeWidth = 1.2
        ..color = palette.inkFaint.withValues(alpha: .55);
      canvas.drawLine(Offset(cx - 12 - w, y), Offset(cx - 12, y), p);
      canvas.drawLine(Offset(cx + 12, y), Offset(cx + 12 + w, y), p);
    }

    final cap = Rect.fromLTWH(6, capY, size.width - 12, capH);
    canvas.drawRRect(
      RRect.fromRectAndRadius(cap, const Radius.circular(5)),
      Paint()
        ..color = palette.shade(.75)
        ..maskFilter = const MaskFilter.blur(BlurStyle.normal, 4),
    );
    canvas.drawRRect(
      RRect.fromRectAndRadius(cap, const Radius.circular(5)),
      Paint()
        ..shader = ui.Gradient.linear(
          Offset(cap.left, cap.top),
          Offset(cap.left, cap.bottom),
          [palette.knobTop, palette.capMid, palette.knobBottom],
          [0, .5, 1],
        ),
    );
    canvas.drawLine(
      Offset(cap.left + 5, cap.top + 1),
      Offset(cap.right - 5, cap.top + 1),
      Paint()
        ..strokeWidth = 1
        ..color = palette.glint(.25),
    );
    final grip = Paint()
      ..strokeWidth = 1.4
      ..color = palette.shade(.6)
      ..strokeCap = StrokeCap.round;
    final gripLight = Paint()
      ..strokeWidth = 1
      ..color = palette.glint(.22)
      ..strokeCap = StrokeCap.round;
    for (var i = -1; i <= 1; i++) {
      final y = cap.center.dy + i * 5;
      canvas.drawLine(Offset(cap.left + 7, y), Offset(cap.right - 7, y), grip);
      canvas.drawLine(Offset(cap.left + 7, y + 1.2), Offset(cap.right - 7, y + 1.2), gripLight);
    }
    canvas.drawLine(
      Offset(cap.left + 2, cap.center.dy),
      Offset(cap.right - 2, cap.center.dy),
      Paint()
        ..strokeWidth = 1.6
        ..color = accent,
    );
  }

  @override
  bool shouldRepaint(_FaderPainter old) => old.t != t || old.accent != accent || old.palette != palette;
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
