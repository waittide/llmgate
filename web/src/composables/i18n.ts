import { computed, watch } from 'vue';
import { settingStore } from '@/stores/setting';

import zhHansMessages from '@/locales/zh_hans.json';
import zhHantMessages from '@/locales/zh_hant.json';
import enMessages from '@/locales/en.json';

const messages: Record<string, unknown> = {
    zh_hans: zhHansMessages,
    zh_hant: zhHantMessages,
    en: enMessages,
};

const languageTags: Record<string, string> = {
    zh_hans: 'zh-Hans',
    zh_hant: 'zh-Hant',
    en: 'en',
};

// 在命名空间字典内按点路径逐级取值（如 'pagination.total'）。
function lookup(dict: unknown, path: string): string | undefined {
    let node: unknown = dict;
    for (const part of path.split('.')) {
        if (node && typeof node === 'object' && part in (node as Record<string, unknown>)) {
            node = (node as Record<string, unknown>)[part];
        } else {
            return undefined;
        }
    }
    return typeof node === 'string' ? node : undefined;
}

export function currentLanguageTag() {
    return languageTags[settingStore.locale] ?? 'zh-Hans';
}

/**
 * useTranslations 对应 React 版 use-intl 的 useTranslations：
 * 返回支持 {name} 插值与点路径键的 t 函数；locale 切换后模板自动更新。
 */
export function useTranslations(namespace: string) {
    const t = (key: string, values?: Record<string, string | number>): string => {
        const dict = messages[settingStore.locale];
        const raw = lookup(dict, `${namespace}.${key}`) ?? `${namespace}.${key}`;
        if (!values) return raw;
        return raw.replace(/\{(\w+)\}/g, (match, name: string) =>
            name in values ? String(values[name]) : match,
        );
    };
    const locale = computed(() => currentLanguageTag());
    return { t, locale };
}

// 应用根组件调用：同步 <html lang>。
export function bindDocumentLocale() {
    document.documentElement.lang = currentLanguageTag();
    watch(
        () => settingStore.locale,
        () => {
            document.documentElement.lang = currentLanguageTag();
        },
    );
}
