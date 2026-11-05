import 'dart:math' as math;

import 'package:flutter/material.dart';

import 'tokens.dart';

/// 凹凸画法：参考图的光从左上打，凸件在右下投影、左上回白，
/// 凹件（屏、槽、井）在左上留内阴影、右下留内高光。
/// 数值只读 [TantrumTokens]，控件里不再散写颜色与偏移。

/// 凸起的底：先亮后暗两层影，再由调用方画本体。
void paintRaise(Canvas canvas, RRect shape, double s, {double depth = 1}) {
  final blur = 3.6 * s * depth;
  canvas.drawRRect(
    shape.shift(Offset(-1.5 * s * depth, -1.7 * s * depth)),
    Paint()
      ..color = TantrumTokens.rimLight
      ..maskFilter = MaskFilter.blur(BlurStyle.normal, blur),
  );
  canvas.drawRRect(
    shape.shift(Offset(1.7 * s * depth, 2.1 * s * depth)),
    Paint()
      ..color = TantrumTokens.castDark
      ..maskFilter = MaskFilter.blur(BlurStyle.normal, blur * 1.25),
  );
}

void paintRaiseCircle(Canvas canvas, Offset center, double r, double s, {double depth = 1}) =>
    paintRaise(
        canvas,
        RRect.fromRectAndRadius(Rect.fromCircle(center: center, radius: r), Radius.circular(r)),
        s,
        depth: depth);

/// 凹陷：把描边环放大后模糊，clip 只留形状内侧，
/// 左上得到内阴影、右下得到内高光。
void paintRecess(Canvas canvas, RRect shape, double s, {double depth = 1}) {
  final band = 2.4 * s * depth;
  canvas.save();
  canvas.clipRRect(shape);
  canvas.drawRRect(
    shape.inflate(band).shift(Offset(1.3 * s * depth, 1.7 * s * depth)),
    Paint()
      ..color = TantrumTokens.insetDark
      ..style = PaintingStyle.stroke
      ..strokeWidth = band * 2.2
      ..maskFilter = MaskFilter.blur(BlurStyle.normal, band),
  );
  canvas.drawRRect(
    shape.inflate(band).shift(Offset(-1.1 * s * depth, -1.3 * s * depth)),
    Paint()
      ..color = TantrumTokens.insetLight
      ..style = PaintingStyle.stroke
      ..strokeWidth = band * 2.2
      ..maskFilter = MaskFilter.blur(BlurStyle.normal, band),
  );
  canvas.restore();
}

void paintRecessCircle(Canvas canvas, Offset center, double r, double s, {double depth = 1}) =>
    paintRecess(
        canvas,
        RRect.fromRectAndRadius(Rect.fromCircle(center: center, radius: r), Radius.circular(r)),
        s,
        depth: depth);

/// 本体斜边：一圈渐变描边——左上回白、右下压暗。
/// 整圈同色描边是"平面贴纸"；用 drawArc 画非圆角矩形又会在面上拉出大弧，
/// 所以统一走 shader 描边，圆形和长条都能用。
void paintBevel(Canvas canvas, RRect shape, double s, {double width = 1.1}) {
  final w = width * s;
  final rect = shape.outerRect;
  final insetRadius = Radius.circular(math.max(0, shape.tlRadiusX - w / 2));
  canvas.drawRRect(
    RRect.fromRectAndRadius(rect.deflate(w / 2), insetRadius),
    Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = w
      ..shader = const LinearGradient(
        begin: Alignment(-0.75, -0.75),
        end: Alignment(0.75, 0.75),
        colors: [
          TantrumTokens.rimLight,
          Color(0x00FFFFFF),
          TantrumTokens.rimShade,
        ],
        stops: [0.0, 0.52, 1.0],
      ).createShader(rect),
  );
}

/// 大面的受光渐变：左上亮、右下暗（面板、屏、旋钮面都靠它撑出体积）。
Shader faceShader(Rect rect, List<Color> colors, {Alignment light = const Alignment(-0.6, -1)}) =>
    LinearGradient(begin: light, end: -light, colors: colors).createShader(rect);
