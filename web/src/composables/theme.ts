import { readonly, ref, watch } from 'vue';

type Theme = 'light' | 'dark' | 'system';
type ResolvedTheme = Exclude<Theme, 'system'>;

function readStoredTheme(): Theme {
    const storedTheme = localStorage.getItem('theme');
    return storedTheme === 'light' || storedTheme === 'dark' || storedTheme === 'system' ? storedTheme : 'system';
}

const theme = ref<Theme>(readStoredTheme());
const systemTheme = ref<ResolvedTheme>(
    window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light',
);

// resolvedTheme 为实际生效主题（system 跟随系统）。
export const resolvedTheme = ref<ResolvedTheme>(
    theme.value === 'system' ? systemTheme.value : theme.value,
);

let started = false;

function applyTheme() {
    resolvedTheme.value = theme.value === 'system' ? systemTheme.value : theme.value;
    const root = document.documentElement;
    root.classList.toggle('dark', resolvedTheme.value === 'dark');
    root.style.colorScheme = resolvedTheme.value;
    document.querySelector('meta[name="theme-color"]')?.setAttribute(
        'content',
        resolvedTheme.value === 'dark' ? '#413a2c' : '#eae9e3',
    );
    localStorage.setItem('theme', theme.value);
}

watch(theme, applyTheme);
// startThemeWatcher 注册系统主题监听与持久化副作用（应用入口调用一次）。
export function startThemeWatcher() {
    if (started) return;
    started = true;
    window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', (event) => {
        systemTheme.value = event.matches ? 'dark' : 'light';
        applyTheme();
    });
    
}


export function setTheme(value: string) {
    if (value === 'light' || value === 'dark' || value === 'system') {
        theme.value = value;
    }
}

export function useTheme() {
    return {
        theme: readonly(theme),
        resolvedTheme: readonly(resolvedTheme),
        setTheme,
    };
}
