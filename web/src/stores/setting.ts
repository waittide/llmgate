import { reactive } from 'vue';

export type Locale = 'zh_hans' | 'zh_hant' | 'en';

const STORAGE_KEY = 'llmgate-settings';

function readPersistedLocale(): Locale {
    try {
        const raw = localStorage.getItem(STORAGE_KEY);
        if (raw) {
            const parsed = JSON.parse(raw) as { state?: { locale?: Locale } };
            const locale = parsed?.state?.locale;
            if (locale === 'zh_hans' || locale === 'zh_hant' || locale === 'en') return locale;
        }
    } catch {
        // ignore storage parse error
    }
    return 'zh_hans';
}

// settingStore 保存界面语言（对应 React 版 useSettingStore，key 与结构保持兼容）。
export const settingStore = reactive({ locale: readPersistedLocale() });

export function setLocale(locale: Locale) {
    settingStore.locale = locale;
    try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify({ state: { locale } }));
    } catch {
        // ignore storage write error
    }
}
