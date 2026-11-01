import 'dart:io' show Platform;
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:window_manager/window_manager.dart';

import '../app_model.dart';
import 'controls.dart';
import 'device.dart';
import 'mobile_device.dart';
import 'siren_binding.dart';
import 'theme.dart';

/// Siren host for the BASSMAKER-derived hardware panel.
/// Windows renders the desktop chassis; Android renders the phone layout.
class SirenPage extends StatefulWidget {
  const SirenPage({super.key, required this.model});

  final AppModel model;

  @override
  State<SirenPage> createState() => _SirenPageState();
}

class _SirenPageState extends State<SirenPage> {
  late final SirenBinding _binding = SirenBinding(widget.model);

  @override
  void dispose() {
    _binding.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: Listenable.merge([widget.model, _binding.visual]),
      builder: (context, _) {
        final palette = _binding.visual.palette;
        final desktop = Platform.isWindows || Platform.isLinux || Platform.isMacOS;
        // Debug preview of the phone chassis without a handset.
        final mobile = !desktop || Platform.environment['SIREN_UI_FORCE_MOBILE'] == '1';
        return Scaffold(
          backgroundColor: palette.deskBottom,
          body: mobile
              ? SafeArea(child: _MobileCanvas(binding: _binding))
              : Column(
                  children: [
                    _CaptionBar(binding: _binding),
                    Expanded(child: _DesktopCanvas(binding: _binding)),
                  ],
                ),
        );
      },
    );
  }
}

class _DesktopCanvas extends StatelessWidget {
  const _DesktopCanvas({required this.binding});

  final SirenBinding binding;

  @override
  Widget build(BuildContext context) {
    final palette = binding.visual.palette;
    return DecoratedBox(
      decoration: BoxDecoration(
        gradient: RadialGradient(
          center: const Alignment(-.3, -.6),
          radius: 1.4,
          colors: [palette.deskTop, palette.deskBottom],
        ),
      ),
      child: LayoutBuilder(
        builder: (context, box) {
          const design = PulsarDevice.design;
          final scale = math
              .min(box.maxWidth / design.width, box.maxHeight / design.height)
              .clamp(.3, 1.6)
              .toDouble();
          return Center(
            child: SizedBox(
              width: design.width * scale,
              height: design.height * scale,
              child: FittedBox(
                fit: BoxFit.fill,
                child: PulsarDevice(model: binding.visual, size: design),
              ),
            ),
          );
        },
      ),
    );
  }
}

class _MobileCanvas extends StatelessWidget {
  const _MobileCanvas({required this.binding});

  final SirenBinding binding;

  @override
  Widget build(BuildContext context) {
    final palette = binding.visual.palette;
    return DecoratedBox(
      decoration: BoxDecoration(
        gradient: RadialGradient(
          center: const Alignment(-.3, -.6),
          radius: 1.4,
          colors: [palette.deskTop, palette.deskBottom],
        ),
      ),
      child: LayoutBuilder(
        builder: (context, box) {
          const design = MobileDevice.design;
          final scale = math
              .min(box.maxWidth / design.width, box.maxHeight / design.height)
              .clamp(.3, 2.4)
              .toDouble();
          return Center(
            child: SizedBox(
              width: design.width * scale,
              height: design.height * scale,
              child: FittedBox(
                fit: BoxFit.fill,
                child: MobileDevice(model: binding.visual, size: design),
              ),
            ),
          );
        },
      ),
    );
  }
}

/// Frameless window caption: drag, theme pills and window buttons.
class _CaptionBar extends StatefulWidget {
  const _CaptionBar({required this.binding});

  final SirenBinding binding;

  @override
  State<_CaptionBar> createState() => _CaptionBarState();
}

class _CaptionBarState extends State<_CaptionBar> {
  DateTime? _lastTapDown;

  void _onPointerDown(PointerDownEvent event) {
    final now = DateTime.now();
    if (_lastTapDown != null &&
        now.difference(_lastTapDown!) < const Duration(milliseconds: 500)) {
      _lastTapDown = null;
      _toggleMaximize();
    } else {
      _lastTapDown = now;
    }
  }

  Future<void> _toggleMaximize() async {
    if (await windowManager.isMaximized()) {
      await windowManager.restore();
    } else {
      await windowManager.maximize();
    }
  }

  @override
  Widget build(BuildContext context) {
    final model = widget.binding.visual;
    final p = model.palette;
    return SizedBox(
      height: 40,
      child: Stack(
        children: [
          Positioned.fill(
            child: DecoratedBox(
              decoration: BoxDecoration(
                gradient: LinearGradient(
                  begin: Alignment.topCenter,
                  end: Alignment.bottomCenter,
                  colors: [p.shellTop, p.shellBottom],
                ),
                border: Border(bottom: BorderSide(color: p.captionRule)),
              ),
            ),
          ),
          const Positioned.fill(child: DragToMoveArea(child: SizedBox.expand())),
          Positioned.fill(
            child: Listener(
              behavior: HitTestBehavior.translucent,
              onPointerDown: _onPointerDown,
              child: Row(
                children: [
                  const SizedBox(width: 14),
                  NeuLed(palette: p, color: p.cyan, on: model.joined, size: 8),
                  const SizedBox(width: 9),
                  Text(
                    'SIREN',
                    style: NeuType.chrome(12.5, p.inkStrong, tracking: 3, w: FontWeight.w800),
                  ),
                  const SizedBox(width: 6),
                  Text(
                    'STUDIO',
                    style: NeuType.chrome(8.5, p.inkFaint, tracking: 2.6, w: FontWeight.w700),
                  ),
                  const SizedBox(width: 10),
                  Text('v0.1', style: NeuType.chrome(10.5, p.inkFaint, tracking: 1)),
                  const Spacer(),
                  NeuWell(
                    size: const Size(108, 26),
                    radius: 13,
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
                            height: 20,
                            fontSize: 11,
                            active: model.theme == e.$1,
                            onTap: () => model.setTheme(e.$1),
                          ),
                      ],
                    ),
                  ),
                  const SizedBox(width: 14),
                  _CaptionButton(palette: p, glyph: _Glyph.minimize, onTap: windowManager.minimize),
                  const SizedBox(width: 4),
                  _CaptionButton(palette: p, glyph: _Glyph.maximize, onTap: _toggleMaximize),
                  const SizedBox(width: 4),
                  _CaptionButton(palette: p, glyph: _Glyph.close, onTap: windowManager.close),
                  const SizedBox(width: 10),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}

enum _Glyph { minimize, maximize, close }

class _CaptionButton extends StatefulWidget {
  const _CaptionButton({required this.palette, required this.glyph, required this.onTap});

  final MetalPalette palette;
  final _Glyph glyph;
  final Future<void> Function() onTap;

  @override
  State<_CaptionButton> createState() => _CaptionButtonState();
}

class _CaptionButtonState extends State<_CaptionButton> {
  bool _hot = false;

  @override
  Widget build(BuildContext context) {
    final p = widget.palette;
    final danger = widget.glyph == _Glyph.close;
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hot = true),
      onExit: (_) => setState(() => _hot = false),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: () => widget.onTap(),
        child: AnimatedContainer(
          duration: const Duration(milliseconds: 110),
          width: 34,
          height: 24,
          decoration: BoxDecoration(
            color: _hot ? (danger ? p.salmon.withValues(alpha: .28) : p.glint(.07)) : null,
            borderRadius: BorderRadius.circular(6),
          ),
          child: CustomPaint(painter: _CaptionPainter(glyph: widget.glyph, color: p.ink)),
        ),
      ),
    );
  }
}

class _CaptionPainter extends CustomPainter {
  _CaptionPainter({required this.glyph, required this.color});

  final _Glyph glyph;
  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    final c = size.center(Offset.zero);
    final p = Paint()
      ..strokeWidth = 1.4
      ..strokeCap = StrokeCap.round
      ..color = color;
    switch (glyph) {
      case _Glyph.minimize:
        canvas.drawLine(Offset(c.dx - 5, c.dy + 3), Offset(c.dx + 5, c.dy + 3), p);
      case _Glyph.maximize:
        canvas.drawRect(
          Rect.fromLTWH(c.dx - 4.5, c.dy - 4.5, 9, 9),
          Paint()
            ..style = PaintingStyle.stroke
            ..strokeWidth = 1.4
            ..color = color,
        );
      case _Glyph.close:
        canvas.drawLine(Offset(c.dx - 4.5, c.dy - 4.5), Offset(c.dx + 4.5, c.dy + 4.5), p);
        canvas.drawLine(Offset(c.dx + 4.5, c.dy - 4.5), Offset(c.dx - 4.5, c.dy + 4.5), p);
    }
  }

  @override
  bool shouldRepaint(_CaptionPainter old) => old.glyph != glyph;
}
