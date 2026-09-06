<script setup lang="ts">
import { LuSun, LuMoon, LuMonitor, LuLanguages } from 'vue-icons-plus/lu';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { useTranslations } from '@/composables/i18n';
import { useTheme } from '@/composables/theme';
import { settingStore, setLocale, type Locale } from '@/stores/setting';

const { t } = useTranslations('setting');
const { theme, setTheme } = useTheme();
</script>

<template>
    <div class="rounded-3xl border border-border bg-card p-6 space-y-5">
        <h2 class="text-lg font-bold text-card-foreground flex items-center gap-2">
            <LuSun class="h-5 w-5" />
            {{ t('appearance') }}
        </h2>

        <!-- 主题 -->
        <div class="flex items-center justify-between gap-4">
            <div class="flex items-center gap-3">
                <LuMoon v-if="theme === 'dark'" class="h-5 w-5 text-muted-foreground" />
                <LuSun v-else class="h-5 w-5 text-muted-foreground" />
                <span class="text-sm font-medium">{{ t('theme.label') }}</span>
            </div>
            <Select :model-value="theme" @update:model-value="(v) => setTheme(v)">
                <SelectTrigger class="w-36 rounded-xl">
                    <SelectValue />
                </SelectTrigger>
                <SelectContent class="rounded-xl">
                    <SelectItem value="light" class="rounded-xl">
                        <LuSun class="size-4" />
                        {{ t('theme.light') }}
                    </SelectItem>
                    <SelectItem value="dark" class="rounded-xl">
                        <LuMoon class="size-4" />
                        {{ t('theme.dark') }}
                    </SelectItem>
                    <SelectItem value="system" class="rounded-xl">
                        <LuMonitor class="size-4" />
                        {{ t('theme.system') }}
                    </SelectItem>
                </SelectContent>
            </Select>
        </div>

        <!-- 语言 -->
        <div class="flex items-center justify-between gap-4">
            <div class="flex items-center gap-3">
                <LuLanguages class="h-5 w-5 text-muted-foreground" />
                <span class="text-sm font-medium">{{ t('language.label') }}</span>
            </div>
            <Select :model-value="settingStore.locale" @update:model-value="(v) => setLocale(v as Locale)">
                <SelectTrigger class="w-36 rounded-xl">
                    <SelectValue />
                </SelectTrigger>
                <SelectContent class="rounded-xl">
                    <SelectItem value="zh_hans" class="rounded-xl">{{ t('language.zh_hans') }}</SelectItem>
                    <SelectItem value="zh_hant" class="rounded-xl">{{ t('language.zh_hant') }}</SelectItem>
                    <SelectItem value="en" class="rounded-xl">{{ t('language.en') }}</SelectItem>
                </SelectContent>
            </Select>
        </div>
    </div>
</template>
