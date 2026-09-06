import { createApp } from 'vue';
import { VueQueryPlugin } from '@tanstack/vue-query';
import App from './App.vue';
import { queryClient } from '@/api/client';
import { bindDocumentLocale } from '@/composables/i18n';
import { startThemeWatcher } from '@/composables/theme';
import './globals.css';

startThemeWatcher();
bindDocumentLocale();

createApp(App).use(VueQueryPlugin, { queryClient }).mount('#root');

// 生产环境在页面加载完成后注册应用根作用域的 Service Worker。
if (import.meta.env.PROD && 'serviceWorker' in navigator) {
    window.addEventListener('load', () => {
        void navigator.serviceWorker.register('/sw.js', { scope: '/' }).catch((error: unknown) => {
            console.error('Service Worker registration failed', error);
        });
    });
}
