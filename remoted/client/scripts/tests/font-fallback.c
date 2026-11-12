/* Exercise the same Fontconfig/Pango registration path used by NSH on Linux,
 * with no installed system fonts available to conceal a missing asset. */
#include <fontconfig/fontconfig.h>
#include <pango/pangocairo.h>
#include <pango/pangofc-fontmap.h>
#include <stdio.h>
#include <string.h>

static int check(PangoContext *context, const char *families, const char *text,
                 const char *expected) {
    PangoLayout *layout = pango_layout_new(context);
    PangoFontDescription *description = pango_font_description_new();
    pango_font_description_set_family(description, families);
    pango_font_description_set_size(description, 14 * PANGO_SCALE);
    pango_layout_set_font_description(layout, description);
    pango_layout_set_text(layout, text, -1);
    int ok = pango_layout_get_unknown_glyphs_count(layout) == 0;
    PangoLayoutIter *iter = pango_layout_get_iter(layout);
    do {
        PangoLayoutRun *run = pango_layout_iter_get_run_readonly(iter);
        if (!run) continue;
        PangoFontDescription *used = pango_font_describe(run->item->analysis.font);
        const char *family = pango_font_description_get_family(used);
        printf("%s => %s\n", text, family);
        ok = ok && strcmp(family, expected) == 0;
        pango_font_description_free(used);
    } while (pango_layout_iter_next_run(iter));
    pango_layout_iter_free(iter);
    pango_font_description_free(description);
    g_object_unref(layout);
    return ok;
}

int main(int argc, char **argv) {
    if (argc != 5) return 2;
    FcConfig *config = FcConfigCreate();
    if (!config) return 2;
    for (int i = 1; i <= 3; ++i) {
        if (!FcConfigAppFontAddFile(config, (const FcChar8 *)argv[i])) return 2;
    }
    FcConfigBuildFonts(config);
    PangoFontMap *map = pango_cairo_font_map_new();
    pango_fc_font_map_set_config(PANGO_FC_FONT_MAP(map), config);
    PangoContext *context = pango_font_map_create_context(map);
    int ok = check(context, argv[4], "Ab09", "Hack Nerd Font Mono");
    ok &= check(context, argv[4], "小橘终端中文", "Maple Mono NF CN");
    ok &= check(context, argv[4], "\xef\x84\xa0", "Hack Nerd Font Mono"); /* U+F120 */
    // Hack already contains all glyphs in this Symbols file. Exercise Symbols
    // as a fallback explicitly by removing Hack from the available chain.
    ok &= check(context, "Missing Font, Symbols Nerd Font Mono, Maple Mono NF CN, monospace",
                "\xef\x84\xa0", "Symbols Nerd Font Mono");
    g_object_unref(context);
    g_object_unref(map);
    FcConfigDestroy(config);
    return ok ? 0 : 1;
}
