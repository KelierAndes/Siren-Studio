import 'package:flutter/material.dart';

import 'app_model.dart';
import 'pages/mesh_page.dart';
import 'rust_api.dart';
import 'theme.dart';
import 'widgets.dart';

class HomeShell extends StatefulWidget {
  const HomeShell({super.key, required this.model});

  final AppModel model;

  @override
  State<HomeShell> createState() => _HomeShellState();
}

class _HomeShellState extends State<HomeShell> {
  @override
  Widget build(BuildContext context) {
    final model = widget.model;
    return Scaffold(
      body: AnimatedBuilder(
        animation: model,
        builder: (context, _) {
          return Stack(
            children: [
              const Positioned.fill(child: ColoredBox(color: AppColors.bg)),
              Positioned(
                left: -220,
                top: -320,
                child: IgnorePointer(
                  child: Container(
                    width: 720,
                    height: 720,
                    decoration: BoxDecoration(
                      shape: BoxShape.circle,
                      gradient: RadialGradient(
                        colors: [
                          AppColors.accent.withValues(alpha: 0.055),
                          Colors.transparent,
                        ],
                      ),
                    ),
                  ),
                ),
              ),
              Positioned(
                right: -180,
                bottom: -260,
                child: IgnorePointer(
                  child: Container(
                    width: 620,
                    height: 620,
                    decoration: BoxDecoration(
                      shape: BoxShape.circle,
                      gradient: RadialGradient(
                        colors: [
                          AppColors.accent2.withValues(alpha: 0.04),
                          Colors.transparent,
                        ],
                      ),
                    ),
                  ),
                ),
              ),
              Positioned.fill(
                child: Row(
                  children: [
                    _Sidebar(model: model),
                    Expanded(
                      child: Column(
                        children: [
                          _TopBar(
                            title: '组网',
                            subtitle: '去中心化 · 全员互收互发',
                            model: model,
                          ),
                          if (model.lastError != null)
                            _ErrorBanner(
                              message: model.lastError!,
                              onClose: model.clearError,
                            ),
                          Expanded(child: MeshPage(model: model)),
                          _LogStrip(model: model),
                        ],
                      ),
                    ),
                  ],
                ),
              ),
            ],
          );
        },
      ),
    );
  }
}

class _Sidebar extends StatelessWidget {
  const _Sidebar({required this.model});

  final AppModel model;

  @override
  Widget build(BuildContext context) {
    return Container(
      width: 216,
      decoration: BoxDecoration(
        color: AppColors.surface.withValues(alpha: 0.82),
        border: const Border(
          right: BorderSide(color: AppColors.borderSoft),
        ),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(20, 24, 20, 28),
            child: Row(
              children: [
                _LogoMark(),
                const SizedBox(width: 12),
                Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      'SIREN',
                      style: TextStyle(
                        fontSize: 17,
                        fontWeight: FontWeight.w800,
                        letterSpacing: 4.5,
                        color: AppColors.text,
                        height: 1,
                        shadows: [
                          Shadow(
                            color: AppColors.accent.withValues(alpha: 0.3),
                            blurRadius: 12,
                          ),
                        ],
                      ),
                    ),
                    const SizedBox(height: 4),
                    const Text(
                      'STUDIO',
                      style: TextStyle(
                        fontSize: 8.5,
                        fontWeight: FontWeight.w700,
                        letterSpacing: 3.4,
                        color: AppColors.textFaint,
                      ),
                    ),
                  ],
                ),
              ],
            ),
          ),
          _NavItem(
            icon: Icons.lan_rounded,
            label: '组网',
            description: 'MESH',
            selected: true,
            active: model.meshRunning,
            activeColor: AppColors.accent,
            onTap: () {},
          ),
          const Spacer(),
          Padding(
            padding: const EdgeInsets.fromLTRB(20, 12, 20, 22),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Container(
                  padding: const EdgeInsets.all(12),
                  decoration: BoxDecoration(
                    color: AppColors.bg2.withValues(alpha: 0.7),
                    borderRadius: BorderRadius.circular(12),
                    border: Border.all(color: AppColors.borderSoft),
                  ),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Row(
                        children: [
                          Container(
                            width: 6,
                            height: 6,
                            decoration: BoxDecoration(
                              color: AppColors.green,
                              shape: BoxShape.circle,
                              boxShadow: [
                                BoxShadow(
                                  color: AppColors.green.withValues(alpha: 0.5),
                                  blurRadius: 6,
                                ),
                              ],
                            ),
                          ),
                          const SizedBox(width: 7),
                          const Text(
                            'LOCAL NODE',
                            style: TextStyle(
                              color: AppColors.textFaint,
                              fontSize: 9,
                              fontWeight: FontWeight.w700,
                              letterSpacing: 1,
                            ),
                          ),
                        ],
                      ),
                      const SizedBox(height: 8),
                      Text(
                        model.meshRunning ? model.mesh!.bind : 'Standby',
                        overflow: TextOverflow.ellipsis,
                        style: AppText.mono.copyWith(
                          color: AppColors.textDim,
                          fontSize: 11,
                        ),
                      ),
                    ],
                  ),
                ),
                const SizedBox(height: 14),
                Text(
                  'v${rust.version()}  ·  Windows',
                  style: const TextStyle(
                    color: AppColors.textFaint,
                    fontSize: 10,
                    letterSpacing: 0.3,
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _LogoMark extends StatelessWidget {
  @override
  Widget build(BuildContext context) {
    return Container(
      width: 38,
      height: 38,
      decoration: BoxDecoration(
        gradient: const LinearGradient(
          colors: [AppColors.accent, AppColors.accent2],
          begin: Alignment.topLeft,
          end: Alignment.bottomRight,
        ),
        borderRadius: BorderRadius.circular(12),
        boxShadow: [
          BoxShadow(
            color: AppColors.accent.withValues(alpha: 0.28),
            blurRadius: 18,
            offset: const Offset(0, 6),
          ),
        ],
      ),
      child: const Icon(
        Icons.graphic_eq_rounded,
        color: AppColors.ink,
        size: 23,
      ),
    );
  }
}

class _NavItem extends StatefulWidget {
  const _NavItem({
    required this.icon,
    required this.label,
    required this.description,
    required this.selected,
    required this.active,
    required this.activeColor,
    required this.onTap,
  });

  final IconData icon;
  final String label;
  final String description;
  final bool selected;
  final bool active;
  final Color activeColor;
  final VoidCallback onTap;

  @override
  State<_NavItem> createState() => _NavItemState();
}

class _NavItemState extends State<_NavItem> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final background = widget.selected
        ? AppColors.accent.withValues(alpha: 0.09)
        : _hover
            ? Colors.white.withValues(alpha: 0.025)
            : Colors.transparent;
    final foreground = widget.selected ? AppColors.text : AppColors.textDim;

    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 3),
      child: MouseRegion(
        cursor: SystemMouseCursors.click,
        onEnter: (_) => setState(() => _hover = true),
        onExit: (_) => setState(() => _hover = false),
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: widget.onTap,
          child: AnimatedContainer(
            duration: const Duration(milliseconds: 160),
            curve: Curves.easeOut,
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 12),
            decoration: BoxDecoration(
              color: background,
              borderRadius: BorderRadius.circular(11),
              border: Border.all(
                color: widget.selected
                    ? AppColors.accent.withValues(alpha: 0.2)
                    : Colors.transparent,
              ),
            ),
            child: Row(
              children: [
                AnimatedContainer(
                  duration: const Duration(milliseconds: 160),
                  width: 3,
                  height: widget.selected ? 20 : 0,
                  margin: const EdgeInsets.only(right: 11),
                  decoration: BoxDecoration(
                    gradient: const LinearGradient(
                      colors: [AppColors.accent, AppColors.accent2],
                      begin: Alignment.topCenter,
                      end: Alignment.bottomCenter,
                    ),
                    borderRadius: BorderRadius.circular(2),
                  ),
                ),
                Icon(
                  widget.icon,
                  size: 19,
                  color: widget.selected ? AppColors.accent : AppColors.textFaint,
                ),
                const SizedBox(width: 12),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        widget.label,
                        style: TextStyle(
                          color: foreground,
                          fontSize: 13,
                          fontWeight: widget.selected
                              ? FontWeight.w700
                              : FontWeight.w500,
                        ),
                      ),
                      const SizedBox(height: 2),
                      Text(
                        widget.description,
                        style: TextStyle(
                          color: widget.selected
                              ? AppColors.accent.withValues(alpha: 0.55)
                              : AppColors.textFaint.withValues(alpha: 0.7),
                          fontSize: 8.5,
                          fontWeight: FontWeight.w700,
                          letterSpacing: 1.6,
                        ),
                      ),
                    ],
                  ),
                ),
                if (widget.active)
                  Container(
                    width: 6,
                    height: 6,
                    decoration: BoxDecoration(
                      color: widget.activeColor,
                      shape: BoxShape.circle,
                      boxShadow: [
                        BoxShadow(
                          color: widget.activeColor.withValues(alpha: 0.6),
                          blurRadius: 6,
                        ),
                      ],
                    ),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class _TopBar extends StatelessWidget {
  const _TopBar({
    required this.title,
    required this.subtitle,
    required this.model,
  });

  final String title;
  final String subtitle;
  final AppModel model;

  @override
  Widget build(BuildContext context) {
    final mesh = model.mesh;
    final meshColor = mesh?.stopping == true
        ? AppColors.amber
        : mesh?.running == true
            ? AppColors.green
            : model.busyMesh
                ? AppColors.amber
                : AppColors.textFaint;
    final meshLabel = mesh?.stopping == true
        ? '离开组网中'
        : mesh?.running == true
            ? '${mesh!.peers.length + 1} 台在组网'
            : model.busyMesh
                ? '加入组网中'
                : '未组网';

    return Container(
      padding: const EdgeInsets.fromLTRB(28, 22, 28, 14),
      child: Row(
        children: [
          Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                title,
                style: Theme.of(context).textTheme.headlineSmall,
              ),
              const SizedBox(height: 3),
              Text(subtitle, style: Theme.of(context).textTheme.bodySmall),
            ],
          ),
          const Spacer(),
          StatusPill(label: meshLabel, color: meshColor),
        ],
      ),
    );
  }
}

class _ErrorBanner extends StatelessWidget {
  const _ErrorBanner({required this.message, required this.onClose});

  final String message;
  final VoidCallback onClose;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(28, 0, 28, 10),
      child: Container(
        padding: const EdgeInsets.fromLTRB(14, 10, 8, 10),
        decoration: BoxDecoration(
          color: AppColors.rose.withValues(alpha: 0.08),
          borderRadius: BorderRadius.circular(10),
          border: Border.all(color: AppColors.rose.withValues(alpha: 0.28)),
        ),
        child: Row(
          children: [
            const Icon(
              Icons.warning_amber_rounded,
              size: 18,
              color: AppColors.rose,
            ),
            const SizedBox(width: 10),
            Expanded(
              child: Text(
                message,
                style: const TextStyle(
                  color: AppColors.rose,
                  fontSize: 12,
                  height: 1.35,
                ),
              ),
            ),
            IconButton(
              onPressed: onClose,
              icon: const Icon(Icons.close_rounded, size: 16),
              color: AppColors.rose,
              visualDensity: VisualDensity.compact,
            ),
          ],
        ),
      ),
    );
  }
}

class _LogStrip extends StatelessWidget {
  const _LogStrip({required this.model});

  final AppModel model;

  @override
  Widget build(BuildContext context) {
    final entry = model.logs.isEmpty ? null : model.logs.last;
    final color = entry?.isError == true ? AppColors.rose : AppColors.green;
    final time = entry == null
        ? '--:--:--'
        : '${entry.at.hour.toString().padLeft(2, '0')}:'
            '${entry.at.minute.toString().padLeft(2, '0')}:'
            '${entry.at.second.toString().padLeft(2, '0')}';

    return Container(
      height: 34,
      padding: const EdgeInsets.symmetric(horizontal: 28),
      decoration: const BoxDecoration(
        color: AppColors.surface,
        border: Border(top: BorderSide(color: AppColors.borderSoft)),
      ),
      child: Row(
        children: [
          Container(
            width: 6,
            height: 6,
            decoration: BoxDecoration(color: color, shape: BoxShape.circle),
          ),
          const SizedBox(width: 9),
          Text(
            time,
            style: AppText.mono.copyWith(
              color: AppColors.textFaint,
              fontSize: 10,
            ),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Text(
              entry?.message ?? '就绪',
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                color: entry?.isError == true
                    ? AppColors.rose
                    : AppColors.textDim,
                fontSize: 11,
              ),
            ),
          ),
          const SizedBox(width: 16),
          Text(
            '48 kHz · stereo · f32',
            style: AppText.mono.copyWith(
              color: AppColors.textFaint.withValues(alpha: 0.75),
              fontSize: 9.5,
            ),
          ),
        ],
      ),
    );
  }
}
