import 'package:flutter/material.dart';

import 'controls.dart';
import 'model.dart';
import 'panels.dart';
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
            child: TopStatusPill(model: model, palette: p),
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
                            LocalCard(model: model, palette: palette),
                            const SizedBox(width: 10),
                            MeshCard(model: model, palette: palette),
                          ],
                        ),
                        const SizedBox(height: 13),
                        BottomStrip(model: model, palette: palette),
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
          Text(
            label,
            style: NeuType.chrome(
              11,
              palette.ink,
              tracking: 2,
              w: FontWeight.w700,
            ),
          ),
          Text(value.toStringAsFixed(1), style: NeuType.numeric(10.5, accent)),
        ],
      ),
    );
  }
}
