<script setup lang="ts">
import { computed, ref } from 'vue';
import { LuCalendarDays, LuCheck, LuLoader, LuX } from 'vue-icons-plus/lu';
import { Input } from '@/components/ui/input';
import { Calendar } from '@/components/ui/calendar';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { Switch } from '@/components/ui/switch';
import { Badge } from '@/components/ui/badge';
import { useGroupList } from '@/api/group';
import type { APIKey } from '@/api/apikey';
import { useTranslations } from '@/composables/i18n';
import { cn } from '@/lib/utils';

export type APIKeyFormData = Omit<APIKey, 'id'>;

function toExpireAt(date: Date, time: string): number {
    const t = /^\d{2}:\d{2}$/.test(time) ? time : '23:59';
    const [hh, mm] = t.split(':').map(Number);
    const d = new Date(date.getFullYear(), date.getMonth(), date.getDate(), hh, mm, 59);
    return Math.floor(d.getTime() / 1000);
}

function parseExpireDate(expireAt?: number): Date | undefined {
    if (!expireAt) return undefined;
    const d = new Date(expireAt * 1000);
    return isNaN(d.getTime()) ? undefined : d;
}

function normalizeHHmm(input: string): string {
    const cleaned = input.replace(/[^\d:]/g, '');
    const parts = cleaned.includes(':') ? cleaned.split(':') : [cleaned.slice(0, 2), cleaned.slice(2, 4)];
    const hh = Math.min(23, Math.max(0, parseInt(parts[0] || '0', 10)));
    const mm = Math.min(59, Math.max(0, parseInt(parts[1] || '0', 10)));
    return `${hh.toString().padStart(2, '0')}:${mm.toString().padStart(2, '0')}`;
}

function normalizeMoneyInput(input: string): string {
    const cleaned = input.replace(/[^\d.]/g, '');
    const [intPart, ...rest] = cleaned.split('.');
    return rest.length > 0 ? `${intPart}.${rest.join('').slice(0, 6)}` : intPart;
}

function toggleModel(current: string | undefined, model: string): string | undefined {
    const models = current ? current.split(',').filter(Boolean) : [];
    const next = models.includes(model)
        ? models.filter((m) => m !== model)
        : [...models, model];
    return next.length ? next.join(',') : undefined;
}

const props = defineProps<{
    apiKey?: APIKey;
    isPending: boolean;
    submitLabel: string;
}>();

const emit = defineEmits<{
    (e: 'submit', data: APIKeyFormData): void;
    (e: 'close'): void;
}>();

const { t } = useTranslations('setting');
const { data: groups = ref([]) } = useGroupList();

const form = ref<APIKeyFormData>({
    name: props.apiKey?.name ?? '',
    api_key: props.apiKey?.api_key ?? '',
    enabled: props.apiKey?.enabled ?? true,
    expire_at: props.apiKey?.expire_at,
    max_cost: props.apiKey?.max_cost,
    supported_models: props.apiKey?.supported_models,
});

const maxCostInput = ref(props.apiKey?.max_cost != null ? String(props.apiKey.max_cost) : '');

const getInitialExpireTime = () => {
    if (props.apiKey?.expire_at) {
        const d = new Date(props.apiKey.expire_at * 1000);
        if (!isNaN(d.getTime())) {
            return `${d.getHours().toString().padStart(2, '0')}:${d.getMinutes().toString().padStart(2, '0')}`;
        }
    }
    return '23:59';
};

const expireTime = ref(getInitialExpireTime());
const expireOpen = ref(false);

const availableModels = computed(() => {
    const list = groups.value ?? [];
    return Array.from(new Set(list.map((g) => g.name).filter(Boolean))).sort((a, b) => a.localeCompare(b));
});

const supportedModels = computed(() => {
    return new Set(form.value.supported_models?.split(',').filter(Boolean));
});

const expireDate = computed(() => parseExpireDate(form.value.expire_at));
const neverExpire = computed(() => !form.value.expire_at);
const isUnlimitedCost = computed(() => maxCostInput.value.trim() === '');

const expireLabel = computed(() => {
    if (neverExpire.value) return t('apiKey.form.neverExpire');
    if (expireDate.value) return expireDate.value.toLocaleDateString();
    return t('apiKey.form.selectDate');
});

const updateForm = (updater: Partial<APIKeyFormData>) => {
    form.value = { ...form.value, ...updater };
};

const handleSelectDate = (d: Date | undefined) => {
    if (d) {
        const time = expireTime.value || '23:59';
        updateForm({ expire_at: toExpireAt(d, time) });
        expireOpen.value = false;
    } else {
        updateForm({ expire_at: undefined });
    }
};

const handleTimeBlur = () => {
    if (!expireDate.value) return;
    const normalized = normalizeHHmm(expireTime.value);
    expireTime.value = normalized;
    updateForm({ expire_at: toExpireAt(expireDate.value, normalized) });
};

const handleToggleNeverExpire = () => {
    if (neverExpire.value) {
        const defaultFuture = new Date();
        defaultFuture.setDate(defaultFuture.getDate() + 30);
        const time = expireTime.value || '23:59';
        expireTime.value = time;
        updateForm({ expire_at: toExpireAt(defaultFuture, time) });
    } else {
        updateForm({ expire_at: undefined });
        expireOpen.value = false;
    }
};

const handleMaxCostChange = (val: string | number | undefined) => {
    const str = val == null ? '' : String(val);
    const normalized = normalizeMoneyInput(str);
    maxCostInput.value = normalized;
    const num = parseFloat(normalized);
    updateForm({ max_cost: Number.isFinite(num) ? num : undefined });
};

const handleClearMaxCost = () => {
    maxCostInput.value = '';
    updateForm({ max_cost: undefined });
};

const handleSubmit = () => {
    if (!form.value.name.trim()) return;
    emit('submit', form.value);
};
</script>

<template>
    <form @submit.prevent="handleSubmit" class="grid gap-2">
        <label class="grid gap-1 text-xs text-muted-foreground">
            {{ t('apiKey.form.name') }}
            <Input
                type="text"
                v-model="form.name"
                class="h-9 text-sm rounded-xl"
                :disabled="props.isPending"
                required
            />
        </label>

        <label class="grid gap-1 text-xs text-muted-foreground">
            {{ t('apiKey.form.value') }}
            <Input
                type="text"
                v-model="form.api_key"
                :placeholder="t('apiKey.form.valuePlaceholder')"
                class="h-9 text-sm rounded-xl"
                :disabled="props.isPending"
            />
        </label>

        <div class="grid gap-1 text-xs text-muted-foreground">
            {{ t('apiKey.form.maxCost') }}
            <div class="flex items-center gap-2">
                <div class="relative flex-1">
                    <span class="absolute left-3 top-1/2 -translate-y-1/2 text-sm text-muted-foreground">$</span>
                    <Input
                        type="text"
                        inputmode="decimal"
                        :placeholder="t('apiKey.form.maxCostPlaceholder')"
                        :model-value="maxCostInput"
                        @update:model-value="handleMaxCostChange"
                        class="h-9 text-sm rounded-xl pl-7"
                        :disabled="props.isPending"
                    />
                </div>
                <button
                    type="button"
                    @click="handleClearMaxCost"
                    :disabled="props.isPending"
                    :aria-pressed="isUnlimitedCost"
                    :class="cn(
                        'h-9 px-3 rounded-xl border text-sm transition-colors shrink-0',
                        isUnlimitedCost
                            ? 'bg-primary text-primary-foreground border-primary/30'
                            : 'border-border bg-muted/20 text-foreground hover:bg-muted/30',
                        props.isPending && 'opacity-50 cursor-not-allowed'
                    )"
                >
                    {{ t('apiKey.form.unlimited') }}
                </button>
            </div>
        </div>

        <div class="grid gap-1 text-xs text-muted-foreground">
            {{ t('apiKey.form.expireAt') }}
            <div class="flex items-center gap-2 relative">
                <Popover :open="expireOpen && !neverExpire" @update:open="(v) => (expireOpen = v)">
                    <PopoverTrigger as-child>
                        <button
                            type="button"
                            :disabled="props.isPending || neverExpire"
                            class="h-9 flex-1 flex items-center justify-between gap-2 rounded-xl border border-border bg-muted/20 px-3 text-sm text-foreground transition-colors hover:bg-muted/30 disabled:opacity-50"
                        >
                            <span class="truncate">{{ expireLabel }}</span>
                            <LuCalendarDays class="size-4 text-muted-foreground" />
                        </button>
                    </PopoverTrigger>
                    <PopoverContent
                        align="start"
                        side="bottom"
                        :side-offset="8"
                        class="w-fit rounded-2xl border border-border/60 shadow-xl overflow-hidden bg-card p-0"
                    >
                        <Calendar
                            mode="single"
                            :selected="expireDate"
                            @select="handleSelectDate"
                            :disabled="props.isPending"
                            :class-names="{ today: '' }"
                        />
                    </PopoverContent>
                </Popover>

                <Input
                    type="text"
                    :model-value="expireTime"
                    @update:model-value="(val) => (expireTime = String(val ?? '').replace(/[^\d:]/g, '').slice(0, 5))"
                    @blur="handleTimeBlur"
                    class="h-9 w-[92px] text-sm rounded-xl"
                    :disabled="props.isPending || neverExpire || !expireDate"
                    inputmode="numeric"
                    placeholder="HH:mm"
                />

                <button
                    type="button"
                    @click="handleToggleNeverExpire"
                    :disabled="props.isPending"
                    :aria-pressed="neverExpire"
                    :class="cn(
                        'h-9 px-3 rounded-xl border text-sm transition-colors whitespace-nowrap shrink-0',
                        neverExpire
                            ? 'bg-primary text-primary-foreground border-primary/30'
                            : 'border-border bg-muted/20 text-foreground hover:bg-muted/30',
                        props.isPending && 'opacity-50 cursor-not-allowed'
                    )"
                >
                    {{ t('apiKey.form.neverExpire') }}
                </button>
            </div>
        </div>

        <div class="grid gap-1">
            <div class="text-xs text-muted-foreground">{{ t('apiKey.form.supportedModels') }}</div>
            <div class="max-h-40 overflow-auto rounded-xl p-2">
                <div v-if="availableModels.length === 0" class="text-xs text-muted-foreground py-2 text-center">
                    {{ t('apiKey.form.noModels') }}
                </div>
                <div v-else class="flex flex-wrap gap-2">
                    <button
                        v-for="m in availableModels"
                        :key="m"
                        type="button"
                        :disabled="props.isPending"
                        @click="updateForm({ supported_models: toggleModel(form.supported_models, m) })"
                        class="text-left disabled:opacity-50"
                    >
                        <Badge
                            :variant="supportedModels.has(m) ? 'default' : 'outline'"
                            :class="cn(
                                'cursor-pointer select-none',
                                !supportedModels.has(m) && 'bg-background/40 hover:bg-background/70'
                            )"
                        >
                            {{ m }}
                        </Badge>
                    </button>
                </div>
            </div>
            <div class="text-[11px] text-muted-foreground/80">{{ t('apiKey.form.modelsHint') }}</div>
        </div>

        <div class="flex items-center justify-between pt-1">
            <span class="text-xs text-muted-foreground">{{ t('apiKey.form.enabled') }}</span>
            <Switch
                :model-value="form.enabled ?? true"
                @update:model-value="(checked) => updateForm({ enabled: checked })"
                :disabled="props.isPending"
            />
        </div>

        <div class="flex gap-2 pt-2 mt-3">
            <button
                type="button"
                @click="emit('close')"
                :disabled="props.isPending"
                class="flex-1 h-9 flex items-center justify-center gap-1.5 rounded-xl bg-muted text-muted-foreground text-sm font-medium transition-all hover:bg-muted/80 active:scale-[0.98] disabled:opacity-50"
            >
                <LuX class="size-4" />
                {{ t('apiKey.form.cancel') }}
            </button>
            <button
                type="submit"
                :disabled="props.isPending || !form.name.trim()"
                class="flex-1 h-9 flex items-center justify-center gap-1.5 rounded-xl bg-primary text-primary-foreground text-sm font-medium transition-all hover:bg-primary/90 active:scale-[0.98] disabled:opacity-50"
            >
                <LuLoader v-if="props.isPending" class="size-4 animate-spin" />
                <LuCheck v-else class="size-4" />
                {{ props.submitLabel }}
            </button>
        </div>
    </form>
</template>
