import 'package:flutter/material.dart';

class AppColors {
  static const bg = Color(0xFF080D12);
  static const bg2 = Color(0xFF0B1219);
  static const surface = Color(0xFF101A23);
  static const surface2 = Color(0xFF14212C);
  static const raised = Color(0xFF1A2A37);
  static const border = Color(0xFF213442);
  static const borderSoft = Color(0xFF192834);
  static const text = Color(0xFFE7F1F8);
  static const textDim = Color(0xFF91A7B9);
  static const textFaint = Color(0xFF5E7487);
  static const accent = Color(0xFF4FD1C5);
  static const accent2 = Color(0xFF38BDF8);
  static const violet = Color(0xFFA78BFA);
  static const amber = Color(0xFFF5B84F);
  static const rose = Color(0xFFF87171);
  static const green = Color(0xFF34D399);
  static const ink = Color(0xFF04110F);
}

class AppText {
  static const mono = TextStyle(
    fontFamily: 'Consolas',
    fontFeatures: [FontFeature.tabularFigures()],
  );
}

ThemeData buildAppTheme() {
  const scheme = ColorScheme.dark(
    primary: AppColors.accent,
    onPrimary: AppColors.ink,
    secondary: AppColors.accent2,
    onSecondary: AppColors.ink,
    surface: AppColors.surface,
    onSurface: AppColors.text,
    error: AppColors.rose,
    onError: AppColors.ink,
    outline: AppColors.border,
  );

  return ThemeData(
    useMaterial3: true,
    brightness: Brightness.dark,
    colorScheme: scheme,
    scaffoldBackgroundColor: AppColors.bg,
    fontFamily: 'Segoe UI',
    visualDensity: VisualDensity.standard,
    splashFactory: InkRipple.splashFactory,
    textTheme: const TextTheme(
      headlineSmall: TextStyle(fontSize: 22, fontWeight: FontWeight.w700, letterSpacing: -0.2),
      titleLarge: TextStyle(fontSize: 18, fontWeight: FontWeight.w700),
      titleMedium: TextStyle(fontSize: 15, fontWeight: FontWeight.w600),
      titleSmall: TextStyle(fontSize: 12.5, fontWeight: FontWeight.w600, letterSpacing: 0.2),
      bodyLarge: TextStyle(fontSize: 14, height: 1.45),
      bodyMedium: TextStyle(fontSize: 13, height: 1.45),
      bodySmall: TextStyle(fontSize: 12, height: 1.4, color: AppColors.textDim),
      labelLarge: TextStyle(fontSize: 13, fontWeight: FontWeight.w600),
      labelMedium: TextStyle(fontSize: 11.5, fontWeight: FontWeight.w600, letterSpacing: 0.3),
      labelSmall: TextStyle(fontSize: 10.5, fontWeight: FontWeight.w600, letterSpacing: 0.6),
    ).apply(bodyColor: AppColors.text, displayColor: AppColors.text),
    iconTheme: const IconThemeData(color: AppColors.textDim, size: 20),
    dividerTheme: const DividerThemeData(color: AppColors.borderSoft, thickness: 1, space: 1),
    sliderTheme: SliderThemeData(
      trackHeight: 4,
      activeTrackColor: AppColors.accent,
      inactiveTrackColor: AppColors.border,
      thumbColor: AppColors.text,
      overlayColor: AppColors.accent.withValues(alpha: 0.12),
      thumbShape: const RoundSliderThumbShape(enabledThumbRadius: 7),
      overlayShape: const RoundSliderOverlayShape(overlayRadius: 16),
      trackShape: const RoundedRectSliderTrackShape(),
    ),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: AppColors.bg2,
      isDense: true,
      contentPadding: const EdgeInsets.symmetric(horizontal: 12, vertical: 12),
      hintStyle: const TextStyle(color: AppColors.textFaint, fontSize: 13),
      labelStyle: const TextStyle(color: AppColors.textDim, fontSize: 12),
      prefixIconColor: AppColors.textFaint,
      suffixIconColor: AppColors.textFaint,
      border: OutlineInputBorder(
        borderRadius: BorderRadius.circular(10),
        borderSide: const BorderSide(color: AppColors.border),
      ),
      enabledBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(10),
        borderSide: const BorderSide(color: AppColors.border),
      ),
      focusedBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(10),
        borderSide: BorderSide(color: AppColors.accent.withValues(alpha: 0.7)),
      ),
      disabledBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(10),
        borderSide: const BorderSide(color: AppColors.borderSoft),
      ),
    ),
    filledButtonTheme: FilledButtonThemeData(
      style: FilledButton.styleFrom(
        backgroundColor: AppColors.accent,
        foregroundColor: AppColors.ink,
        disabledBackgroundColor: AppColors.raised,
        disabledForegroundColor: AppColors.textFaint,
        padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 15),
        textStyle: const TextStyle(fontSize: 13, fontWeight: FontWeight.w700),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(11)),
      ),
    ),
    outlinedButtonTheme: OutlinedButtonThemeData(
      style: OutlinedButton.styleFrom(
        foregroundColor: AppColors.text,
        side: const BorderSide(color: AppColors.border),
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
        textStyle: const TextStyle(fontSize: 13, fontWeight: FontWeight.w600),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(11)),
      ),
    ),
    textButtonTheme: TextButtonThemeData(
      style: TextButton.styleFrom(
        foregroundColor: AppColors.accent,
        textStyle: const TextStyle(fontSize: 12.5, fontWeight: FontWeight.w600),
      ),
    ),
    switchTheme: SwitchThemeData(
      thumbColor: WidgetStateProperty.resolveWith(
        (states) => states.contains(WidgetState.selected) ? AppColors.ink : AppColors.textDim,
      ),
      trackColor: WidgetStateProperty.resolveWith(
        (states) => states.contains(WidgetState.selected) ? AppColors.accent : AppColors.raised,
      ),
      trackOutlineColor: const WidgetStatePropertyAll(AppColors.border),
    ),
    segmentedButtonTheme: SegmentedButtonThemeData(
      style: ButtonStyle(
        backgroundColor: WidgetStateProperty.resolveWith(
          (states) => states.contains(WidgetState.selected)
              ? AppColors.accent.withValues(alpha: 0.14)
              : AppColors.bg2,
        ),
        foregroundColor: WidgetStateProperty.resolveWith(
          (states) =>
              states.contains(WidgetState.selected) ? AppColors.accent : AppColors.textDim,
        ),
        side: WidgetStateProperty.resolveWith(
          (states) => BorderSide(
            color: states.contains(WidgetState.selected)
                ? AppColors.accent.withValues(alpha: 0.5)
                : AppColors.border,
          ),
        ),
        textStyle: const WidgetStatePropertyAll(
          TextStyle(fontSize: 12.5, fontWeight: FontWeight.w600),
        ),
        padding: const WidgetStatePropertyAll(
          EdgeInsets.symmetric(horizontal: 14, vertical: 12),
        ),
        shape: WidgetStatePropertyAll(
          RoundedRectangleBorder(borderRadius: BorderRadius.circular(10)),
        ),
      ),
    ),
    tooltipTheme: TooltipThemeData(
      decoration: BoxDecoration(
        color: AppColors.raised,
        borderRadius: BorderRadius.circular(8),
        border: Border.all(color: AppColors.border),
      ),
      textStyle: const TextStyle(color: AppColors.text, fontSize: 12),
      waitDuration: const Duration(milliseconds: 500),
    ),
    scrollbarTheme: ScrollbarThemeData(
      thickness: const WidgetStatePropertyAll(6),
      radius: const Radius.circular(8),
      thumbColor: WidgetStatePropertyAll(AppColors.textFaint.withValues(alpha: 0.4)),
    ),
  );
}
