<script setup lang="ts">
import { computed } from 'vue'
import dayjs from 'dayjs'
import { useTranslations } from '@/composables/i18n'
import { useTheme } from '@/composables/theme'
import { toast } from '@/composables/toast'
import { useAPIKeyDashboardStats } from '@/api/apikey'
import { logout } from '@/stores/auth'
import { settingStore, setLocale } from '@/stores/setting'
import AnimatedNumber from '@/components/common/AnimatedNumber.vue'
import CopyIconButton from '@/components/common/CopyButton.vue'
import { Button } from '@/components/ui/button'
import { Progress } from '@/components/ui/progress'
import {
    LuArrowDownToLine,
    LuArrowUpFromLine,
    LuDollarSign,
    LuCheckCircle,
    LuXCircle,
    LuKeyRound,
    LuLogOut,
    LuCalendar,
    LuWallet,
    LuSun,
    LuMoon,
    LuLanguages,
    LuZap,
    LuLayers,
    LuClock,
} from 'vue-icons-plus/lu'

const { t } = useTranslations('apiKeyDashboard')
const { data, error } = useAPIKeyDashboardStats()
const { resolvedTheme, setTheme } = useTheme()

async function copyToClipboard(str: string): Promise<void> {
    if (navigator.clipboard && window.isSecureContext) {
        await navigator.clipboard.writeText(str)
        return
    }
    const textArea = document.createElement('textarea')
    textArea.value = str
    textArea.style.position = 'fixed'
    textArea.style.left = '-999999px'
    textArea.style.top = '-999999px'
    document.body.appendChild(textArea)
    textArea.focus()
    textArea.select()
    const successful = document.execCommand('copy')
    textArea.remove()
    if (!successful) {
        throw new Error('execCommand copy failed')
    }
}

async function copyWithToast(text: string, label: string) {
    try {
        await copyToClipboard(text)
        toast.success(`${label} copied`)
    } catch {
        toast.error(t('error'))
    }
}

// Quota calculations
const usedCost = computed(() => data.value?.stats.total_cost.raw ?? 0)
const maxCost = computed(() => data.value?.info.max_cost || 0)
const progressValue = computed(() => (maxCost.value > 0 ? Math.min(100, (usedCost.value / maxCost.value) * 100) : 0))

// Expiry calculations
const expireAt = computed(() => (data.value?.info.expire_at ? dayjs.unix(data.value.info.expire_at) : null))
const isExpired = computed(() => (expireAt.value ? expireAt.value.isBefore(dayjs()) : false))
const daysUntilExpire = computed(() => (expireAt.value ? expireAt.value.diff(dayjs(), 'day') : null))

const supportedModels = computed(() => {
    const models = data.value?.info.supported_models
    return models
        ? models
              .split(',')
              .map((m) => m.trim())
              .filter(Boolean)
        : []
})

const maskedApiKey = computed(() => {
    const key = data.value?.info.api_key
    if (!key) return ''
    return `${key.slice(0, 11)}********${key.slice(-4)}`
})

const toggleTheme = () => setTheme(resolvedTheme.value === 'dark' ? 'light' : 'dark')
const toggleLanguage = () => {
    if (settingStore.locale === 'zh_hans') setLocale('zh_hant')
    else if (settingStore.locale === 'zh_hant') setLocale('en')
    else setLocale('zh_hans')
}
</script>

<template>
    <div v-if="error || !data" class="min-h-screen flex items-center justify-center">
        <div class="text-center space-y-4">
            <p class="text-destructive font-medium">{{ t('error') }}</p>
            <Button @click="logout" variant="outline" class="rounded-xl">
                {{ t('logout') }}
            </Button>
        </div>
    </div>

    <div v-else class="mx-auto max-w-6xl px-3 md:px-6">
        <!-- Header - Consistent with app.tsx -->
        <header class="my-6 flex items-center gap-2 px-2">
            <h1 class="ml-2 flex-1 truncate text-2xl font-bold tracking-tight">LLMGate</h1>
            <div class="flex items-center gap-2">
                <Button variant="ghost" size="icon" @click="toggleTheme" class="rounded-xl hover:bg-accent">
                    <LuSun class="size-4 rotate-0 scale-100 transition-all dark:-rotate-90 dark:scale-0" />
                    <LuMoon class="absolute size-4 rotate-90 scale-0 transition-all dark:rotate-0 dark:scale-100" />
                </Button>
                <Button variant="ghost" size="icon" @click="toggleLanguage" class="rounded-xl hover:bg-accent">
                    <LuLanguages class="size-4" />
                </Button>
                <div class="w-px h-6 bg-border mx-1" />
                <Button variant="ghost" size="icon" @click="logout" class="rounded-xl hover:bg-destructive/10 hover:text-destructive">
                    <LuLogOut class="size-4" />
                </Button>
            </div>
        </header>

        <main class="mb-10">
            <div class="space-y-6">
                <!-- Hero: Identity + Limits -->
                <div class="overflow-hidden rounded-3xl border bg-card">
                    <div class="grid grid-cols-1 md:grid-cols-2">
                        <!-- Left: Key Info -->
                        <div class="p-6 md:p-8 flex flex-col relative">
                            <LuKeyRound aria-hidden="true" class="pointer-events-none absolute top-6 right-6 h-27 w-27 text-muted-foreground/10" />
                            <h2 class="text-2xl font-bold truncate pr-16">{{ data.info.name }}</h2>
                            <div class="mt-4 flex items-center gap-2 rounded-xl border border-border/50 bg-muted/50 p-3">
                                <code class="flex-1 font-mono text-sm truncate">
                                    {{ maskedApiKey }}
                                </code>
                                <CopyIconButton
                                    :text="data.info.api_key"
                                    class-name="flex size-8 items-center justify-center rounded-lg bg-primary/10 text-primary transition-all hover:bg-primary hover:text-primary-foreground active:scale-95"
                                    copy-icon-class-name="size-4"
                                    check-icon-class-name="size-4"
                                />
                            </div>
                            <!-- Expiry & Quota inline -->
                            <div class="mt-auto pt-6 text-sm">
                                <div class="flex items-center justify-between">
                                    <span class="flex items-center gap-2 text-muted-foreground">
                                        <LuCalendar class="w-4 h-4" />
                                        {{ t('expireAt') }}
                                    </span>
                                    <span v-if="expireAt" :class="['font-medium', isExpired ? 'text-destructive' : '']">
                                        {{ expireAt.format('YYYY-MM-DD') }}
                                        <span v-if="!isExpired && daysUntilExpire !== null" class="ml-2 text-xs bg-secondary px-2 py-0.5 rounded-full">
                                            {{ daysUntilExpire }} {{ t('daysLeft') }}
                                        </span>
                                        <span v-if="isExpired" class="ml-2 text-xs bg-destructive/10 text-destructive px-2 py-0.5 rounded-full">
                                            {{ t('expired') }}
                                        </span>
                                    </span>
                                    <span v-else class="font-medium">{{ t('neverExpire') }}</span>
                                </div>
                            </div>
                        </div>
                        <!-- Right: Quota visual -->
                        <div class="relative flex flex-col justify-center border-t bg-muted/30 p-6 md:border-l md:border-t-0 md:p-8">
                            <LuWallet aria-hidden="true" class="pointer-events-none absolute top-6 right-6 h-27 w-27 text-muted-foreground/10" />
                            <div class="text-lg text-muted-foreground uppercase tracking-wider mb-2">{{ t('totalCost') }}</div>
                            <div class="text-6xl font-bold text-chart-1">
                                <AnimatedNumber :value="data.stats.total_cost.formatted.value" />
                                <span class="text-lg font-normal text-muted-foreground ml-1">{{ data.stats.total_cost.formatted.unit }}</span>
                            </div>
                            <div v-if="maxCost > 0" class="mt-4">
                                <Progress :value="progressValue" class="h-4 *:data-[slot=progress-indicator]:bg-chart-1" />
                                <div class="flex justify-between text-sm text-muted-foreground mt-1">
                                    <span>0</span>
                                    <span>{{ maxCost.toFixed(2) }} $</span>
                                </div>
                            </div>
                        </div>
                    </div>
                </div>

                <!-- Row 2: Request Health -->
                <div class="grid grid-cols-2 gap-4 md:grid-cols-4">
                    <div class="rounded-2xl border bg-card p-5">
                        <div class="mb-3 flex items-center gap-2 text-xs text-muted-foreground">
                            <LuCheckCircle class="size-4 text-chart-2" />
                            {{ t('successRequests') }}
                        </div>
                        <div class="text-2xl font-bold">
                            <AnimatedNumber :value="data.stats.request_success.formatted.value" />
                            <span class="ml-1 text-sm font-normal text-muted-foreground">{{ data.stats.request_success.formatted.unit }}</span>
                        </div>
                    </div>

                    <div class="rounded-2xl border bg-card p-5">
                        <div class="mb-3 flex items-center gap-2 text-xs text-muted-foreground">
                            <LuXCircle class="size-4 text-destructive" />
                            {{ t('failedRequests') }}
                        </div>
                        <div class="text-2xl font-bold">
                            <AnimatedNumber :value="data.stats.request_failed.formatted.value" />
                            <span class="ml-1 text-sm font-normal text-muted-foreground">{{ data.stats.request_failed.formatted.unit }}</span>
                        </div>
                    </div>

                    <div class="rounded-2xl border bg-card p-5">
                        <div class="mb-3 flex items-center gap-2 text-xs text-muted-foreground">
                            <LuZap class="size-4 text-chart-4" />
                            {{ t('requestCount') }}
                        </div>
                        <div class="text-2xl font-bold">
                            <AnimatedNumber :value="data.stats.request_count.formatted.value" />
                            <span class="ml-1 text-sm font-normal text-muted-foreground">{{ data.stats.request_count.formatted.unit }}</span>
                        </div>
                    </div>

                    <div class="rounded-2xl border bg-card p-5">
                        <div class="mb-3 flex items-center gap-2 text-xs text-muted-foreground">
                            <LuClock class="size-4 text-chart-5" />
                            {{ t('timeConsumed') }}
                        </div>
                        <div class="text-2xl font-bold">
                            <AnimatedNumber :value="data.stats.wait_time.formatted.value" />
                            <span class="ml-1 text-sm font-normal text-muted-foreground">{{ data.stats.wait_time.formatted.unit }}</span>
                        </div>
                    </div>
                </div>

                <!-- Row 3: Token & Cost breakdown -->
                <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
                    <!-- Token breakdown -->
                    <div class="rounded-2xl border bg-card p-6">
                        <div class="flex items-center gap-2 mb-4">
                            <LuZap class="w-5 h-5 text-chart-4" />
                            <span class="font-semibold">{{ t('totalToken') }}</span>
                            <span class="ml-auto text-2xl font-bold">
                                <AnimatedNumber :value="data.stats.total_token.formatted.value" />
                                <span class="text-sm font-normal text-muted-foreground ml-1">{{ data.stats.total_token.formatted.unit }}</span>
                            </span>
                        </div>
                        <div class="grid grid-cols-2 gap-4 pt-4 border-t border-border/50">
                            <div>
                                <div class="flex items-center gap-1.5 text-xs text-muted-foreground mb-1">
                                    <LuArrowDownToLine class="w-3.5 h-3.5" />
                                    {{ t('inputTokens') }}
                                </div>
                                <div class="text-lg font-semibold">
                                    <AnimatedNumber :value="data.stats.input_token.formatted.value" />
                                    <span class="text-xs font-normal text-muted-foreground ml-1">{{ data.stats.input_token.formatted.unit }}</span>
                                </div>
                            </div>
                            <div>
                                <div class="flex items-center gap-1.5 text-xs text-muted-foreground mb-1">
                                    <LuArrowUpFromLine class="w-3.5 h-3.5" />
                                    {{ t('outputTokens') }}
                                </div>
                                <div class="text-lg font-semibold">
                                    <AnimatedNumber :value="data.stats.output_token.formatted.value" />
                                    <span class="text-xs font-normal text-muted-foreground ml-1">{{ data.stats.output_token.formatted.unit }}</span>
                                </div>
                            </div>
                        </div>
                    </div>
                    <!-- Cost breakdown -->
                    <div class="rounded-2xl border bg-card p-6">
                        <div class="flex items-center gap-2 mb-4">
                            <LuDollarSign class="w-5 h-5 text-chart-1" />
                            <span class="font-semibold">{{ t('totalCost') }}</span>
                            <span class="ml-auto text-2xl font-bold">
                                <AnimatedNumber :value="data.stats.total_cost.formatted.value" />
                                <span class="text-sm font-normal text-muted-foreground ml-1">{{ data.stats.total_cost.formatted.unit }}</span>
                            </span>
                        </div>
                        <div class="grid grid-cols-2 gap-4 pt-4 border-t border-border/50">
                            <div>
                                <div class="flex items-center gap-1.5 text-xs text-muted-foreground mb-1">
                                    <LuArrowDownToLine class="w-3.5 h-3.5" />
                                    {{ t('inputCost') }}
                                </div>
                                <div class="text-lg font-semibold">
                                    <AnimatedNumber :value="data.stats.input_cost.formatted.value" />
                                    <span class="text-xs font-normal text-muted-foreground ml-1">{{ data.stats.input_cost.formatted.unit }}</span>
                                </div>
                            </div>
                            <div>
                                <div class="flex items-center gap-1.5 text-xs text-muted-foreground mb-1">
                                    <LuArrowUpFromLine class="w-3.5 h-3.5" />
                                    {{ t('outputCost') }}
                                </div>
                                <div class="text-lg font-semibold">
                                    <AnimatedNumber :value="data.stats.output_cost.formatted.value" />
                                    <span class="text-xs font-normal text-muted-foreground ml-1">{{ data.stats.output_cost.formatted.unit }}</span>
                                </div>
                            </div>
                        </div>
                    </div>
                </div>

                <!-- Supported Models -->
                <div v-if="supportedModels.length > 0" class="rounded-2xl border bg-card p-6">
                    <div class="flex items-center gap-2 mb-4">
                        <LuLayers class="w-5 h-5 text-chart-3" />
                        <span class="font-semibold">{{ t('supportedModels') }}</span>
                    </div>
                    <div class="flex flex-wrap gap-2">
                        <Button
                            v-for="model in supportedModels"
                            :key="model"
                            variant="secondary"
                            size="sm"
                            class="h-8 rounded-lg px-3 text-sm transition-colors hover:bg-primary hover:text-primary-foreground"
                            @click="copyWithToast(model, model)"
                        >
                            {{ model }}
                        </Button>
                    </div>
                </div>
            </div>
        </main>
    </div>
</template>
