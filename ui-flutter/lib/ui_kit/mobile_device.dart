import 'package:flutter/material.dart';

import 'controls.dart';
import 'model.dart';
import 'panels.dart';
import 'screen_display.dart';
import 'stream_card.dart';
import 'theme.dart';

/// 手机版：与桌面同一套结构与卡片（顶栏状态胶囊 + EQ 屏 + 设备卡 + 系统卡 + 底栏），
/// 只是把「四卡一排」改成纵向堆叠并可滚动，卡片内部排布与桌面一致。
class MobileDevice extends StatelessWidget {
  const MobileDevice({super.key, required this.model, required this.size});

  static const Size design = Size(390, 844);
  final ShaperModel model;
  final Size size;

  MetalPalette get palette => model.palette;

  @override
  Widget build(BuildContext context) {
    final m = model;
    const w = 362.0;
    return ListenableBuilder(
      listenable: m,
      builder: (context, _) {
        final p = palette;
        return Container(
          width: size.width,
          height: size.height,
          decoration: BoxDecoration(
            gradient: RadialGradient(
              center: const Alignment(-.3, -.6),
              radius: 1.4,
              colors: [p.deskTop, p.deskBottom],
            ),
          ),
          child: Padding(
            padding: const EdgeInsets.fromLTRB(14, 12, 14, 12),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                SizedBox(
                  height: 30,
                  child: Row(
                    children: [
                      NeuLed(palette: p, color: p.cyan, on: m.joined, size: 8),
                      const SizedBox(width: 9),
                      Text(
                        'SIREN',
                        style: NeuType.chrome(14, p.inkStrong, tracking: 3, w: FontWeight.w800),
                      ),
                      const Spacer(),
                      NeuWell(
                        size: const Size(108, 26),
                        radius: 13,
                        palette: p,
                        tint: p.wellTint,
                        padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 3),
                        child: Row(
                          children: [
                            for (final option in const [
                              (AppThemeMode.dark, '暗色'),
                              (AppThemeMode.light, '亮色'),
                            ])
                              NeuPill(
                                palette: p,
                                label: option.$2,
                                height: 20,
                                fontSize: 11,
                                active: m.theme == option.$1,
                                onTap: () => m.setTheme(option.$1),
                              ),
                          ],
                        ),
                      ),
                    ],
                  ),
                ),
                const SizedBox(height: 8),
                Center(child: TopStatusPill(model: m, palette: p, width: w)),
                const SizedBox(height: 10),
                ScreenDisplay(model: m, palette: p, radius: 14, size: const Size(w, 190)),
                const SizedBox(height: 10),
                _MeterRow(model: m, palette: p),
                const SizedBox(height: 10),
                Expanded(
                  child: SingleChildScrollView(
                    child: Column(
                      children: [
                        StreamCard(channel: m.channels[0], palette: p, size: const Size(w, 232)),
                        const SizedBox(height: 10),
                        StreamCard(channel: m.channels[1], palette: p, size: const Size(w, 232)),
                        const SizedBox(height: 10),
                        LocalCard(model: m, palette: p, size: const Size(w, 210)),
                        const SizedBox(height: 10),
                        MeshCard(model: m, palette: p, size: const Size(w, 170)),
                        const SizedBox(height: 10),
                        BottomStrip(model: m, palette: p, size: const Size(w, 112), twoRow: true),
                        const SizedBox(height: 4),
                      ],
                    ),
                  ),
                ),
              ],
            ),
          ),
        );
      },
    );
  }
}

/// IN / OUT 电平表：桌面是竖推子，手机宽度有限，改成两条横向表。
class _MeterRow extends StatelessWidget {
  const _MeterRow({required this.model, required this.palette});

  final ShaperModel model;
  final MetalPalette palette;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      height: 40,
      child: Row(
        children: [
          Expanded(child: _meter('IN', model.input, palette.cyan)),
          const SizedBox(width: 10),
          Expanded(child: _meter('OUT', model.output, palette.orange)),
        ],
      ),
    );
  }

  Widget _meter(String label, double value, Color accent) {
    final p = palette;
    final norm = ((value + 48) / 60).clamp(0.0, 1.0);
    return NeuWell(
      radius: 10,
      palette: p,
      tint: p.wellTint,
      padding: const EdgeInsets.fromLTRB(10, 6, 10, 6),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              Text(label, style: NeuType.chrome(9.5, p.inkFaint, tracking: 1.2, w: FontWeight.w700)),
              const Spacer(),
              Text('${value.toStringAsFixed(1)} dB', style: NeuType.numeric(10.5, accent)),
            ],
          ),
          const SizedBox(height: 5),
          Expanded(
            child: LayoutBuilder(
              builder: (context, box) => Stack(
                children: [
                  Positioned.fill(
                    child: DecoratedBox(
                      decoration: BoxDecoration(
                        color: p.bevel.withValues(alpha: .16),
                        borderRadius: BorderRadius.circular(3),
                      ),
                    ),
                  ),
                  Positioned(
                    left: 0,
                    top: 0,
                    bottom: 0,
                    width: box.maxWidth * norm,
                    child: DecoratedBox(
                      decoration: BoxDecoration(
                        gradient: LinearGradient(colors: [accent.withValues(alpha: .5), accent]),
                        borderRadius: BorderRadius.circular(3),
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}
