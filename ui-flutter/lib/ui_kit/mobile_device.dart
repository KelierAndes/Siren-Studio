import 'package:flutter/material.dart';

import 'controls.dart';
import 'device.dart' show IconButton32;
import 'model.dart';
import 'module_block.dart';
import 'screen_display.dart';
import 'theme.dart';

/// 手机版：390 x 844 比例，同一套深色金属语言的纵向排布
class MobileDevice extends StatefulWidget {
  const MobileDevice({super.key, required this.model, required this.size});

  static const Size design = Size(390, 844);
  final ShaperModel model;
  final Size size;

  @override
  State<MobileDevice> createState() => _MobileDeviceState();
}

class _MobileDeviceState extends State<MobileDevice> with SingleTickerProviderStateMixin {
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
        const w = 362.0;
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
          child: Padding(
            padding: const EdgeInsets.fromLTRB(14, 46, 14, 12),
            child: SizedBox(
              width: w,
              height: 786,
              child: Column(
                children: [
                  SizedBox(
                    width: w,
                    height: 36,
                    child: Row(
                      children: [
                        NeuLed(palette: palette, color: palette.orange, on: m.power, size: 8),
                        const SizedBox(width: 8),
                        Text(
                          'SIREN',
                          style: NeuType.chrome(14, palette.inkStrong, tracking: 3, w: FontWeight.w700),
                        ),
                        const SizedBox(width: 10),
                        Expanded(
                          child: Text(
                            m.statusText,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: NeuType.chrome(9.5, palette.inkFaint, tracking: .6),
                          ),
                        ),
                        IconButton32(palette: palette, icon: Icons.settings_outlined, size: 28, onTap: m.reset),
                      ],
                    ),
                  ),
                  const SizedBox(height: 10),
                  ScreenDisplay(
                    model: m,
                    palette: palette,
                    radius: 14,
                    size: const Size(w, 230),
                  ),
                  const SizedBox(height: 10),
                  _ModuleTabs(model: m, palette: palette, width: w),
                  const SizedBox(height: 10),
                  Center(
                    child: ModuleBlock(
                      channel: m.channels[m.mobileModule],
                      model: m,
                      palette: palette,
                      clock: _clock,
                      size: const Size(300, 268),
                    ),
                  ),
                  const SizedBox(height: 10),
                  _MobileFaders(model: m, palette: palette, width: w),
                  const SizedBox(height: 10),
                  NeuWell(
                    size: const Size(w, 44),
                    radius: 12,
                    palette: palette,
                    tint: palette.stripTint,
                    padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
                    child: Row(
                      children: [
                        Text('缓冲', style: NeuType.chrome(9.5, palette.ink, tracking: 1.2, w: FontWeight.w700)),
                        const SizedBox(width: 6),
                        Expanded(
                          child: LayoutBuilder(
                            builder: (context, box) => NeuSlider(
                              palette: palette,
                              value: m.hp,
                              range: ShaperModel.hpRange,
                              accentColor: palette.cyan,
                              width: box.maxWidth,
                              height: 22,
                              onChanged: m.setHp,
                            ),
                          ),
                        ),
                        const SizedBox(width: 8),
                        Text('增益', style: NeuType.chrome(9.5, palette.ink, tracking: 1.2, w: FontWeight.w700)),
                        const SizedBox(width: 5),
                        Text(
                          '${m.lp.toStringAsFixed(1)} dB',
                          style: NeuType.numeric(10, palette.cyan),
                        ),
                        const SizedBox(width: 10),
                        NeuPill(
                          palette: palette,
                          label: 'BYPASS',
                          height: 28,
                          fontSize: 10.5,
                          led: true,
                          active: m.bypass,
                          accent: palette.salmon,
                          onTap: m.toggleBypass,
                        ),
                      ],
                    ),
                  ),
                ],
              ),
            ),
          ),
        );
      },
    );
  }
}

class _ModuleTabs extends StatelessWidget {
  const _ModuleTabs({required this.model, required this.palette, required this.width});
  final ShaperModel model;
  final MetalPalette palette;
  final double width;

  @override
  Widget build(BuildContext context) {
    final labels = [
      for (final c in model.channels) c.tabTitle,
    ];
    return ListenableBuilder(
      listenable: model,
      builder: (context, _) => NeuWell(
        size: Size(width, 38),
        radius: 14,
        palette: palette,
        tint: palette.wellTint,
        padding: const EdgeInsets.symmetric(horizontal: 5, vertical: 5),
        child: Row(
          children: [
            for (var i = 0; i < labels.length; i++)
              Expanded(
                child: Padding(
                  padding: EdgeInsets.only(right: i == labels.length - 1 ? 0 : 5),
                  child: NeuPill(
                    palette: palette,
                    label: labels[i],
                    height: 28,
                    fontSize: 11,
                    accent: model.channels[i].accentFor(palette),
                    active: model.mobileModule == i,
                    fill: true,
                    onTap: () => model.setMobileModule(i),
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

class _MobileFaders extends StatelessWidget {
  const _MobileFaders({required this.model, required this.palette, required this.width});
  final ShaperModel model;
  final MetalPalette palette;
  final double width;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: model,
      builder: (context, _) => NeuRaised(
        palette: palette,
        size: Size(width, 90),
        radius: 14,
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
        child: Row(
          children: [
            _miniFader('IN', model.input, ShaperModel.inputRange, model.setInput, palette.cyan),
            const SizedBox(width: 14),
            _miniFader('OUT', model.output, ShaperModel.outputRange, model.setOutput, palette.orange),
            const SizedBox(width: 14),
            Expanded(
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Row(
                    children: [
                      SizedBox(
                        width: 30,
                        child: Text('DRY', style: NeuType.chrome(9, palette.inkFaint, tracking: .8)),
                      ),
                      Expanded(
                        child: LayoutBuilder(
                          builder: (context, box) => NeuSlider(
                            palette: palette,
                            value: model.dry,
                            range: ShaperModel.dryWetRange,
                            accentColor: palette.ink,
                            width: box.maxWidth,
                            height: 20,
                            onChanged: model.setDry,
                          ),
                        ),
                      ),
                    ],
                  ),
                  const SizedBox(height: 6),
                  Row(
                    children: [
                      SizedBox(
                        width: 30,
                        child: Text('WET', style: NeuType.chrome(9, palette.inkFaint, tracking: .8)),
                      ),
                      Expanded(
                        child: LayoutBuilder(
                          builder: (context, box) => NeuSlider(
                            palette: palette,
                            value: model.wet,
                            range: ShaperModel.dryWetRange,
                            accentColor: palette.cyan,
                            width: box.maxWidth,
                            height: 20,
                            onChanged: model.setWet,
                          ),
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
    );
  }

  Widget _miniFader(
    String label,
    double value,
    NeuRange range,
    ValueChanged<double> onChanged,
    Color accent,
  ) {
    return Column(
      mainAxisAlignment: MainAxisAlignment.center,
      children: [
        NeuFader(
          palette: palette,
          value: value,
          range: range,
          accent: accent,
          width: 36,
          height: 52,
          ticks: 2,
          onChanged: onChanged,
        ),
        const SizedBox(height: 2),
        Text(label, style: NeuType.chrome(9, palette.inkFaint, tracking: 1.4)),
      ],
    );
  }
}
