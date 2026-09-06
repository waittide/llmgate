<script setup lang="ts">
import { ref, computed } from 'vue';
import { LuChevronDown, LuChevronRight, LuCopy, LuCheck } from 'vue-icons-plus/lu';
import { useTranslations } from '@/composables/i18n';
import { toast } from '@/composables/toast';

interface Props {
    data: any;
    nodeKey?: string | number;
    depth?: number;
    collapsed?: boolean | number;
    isLast?: boolean;
    displayDataTypes?: boolean;
    displayObjectSize?: boolean;
    enableClipboard?: boolean;
}

const props = withDefaults(defineProps<Props>(), {
    nodeKey: undefined,
    depth: 0,
    collapsed: false,
    isLast: true,
    displayDataTypes: false,
    displayObjectSize: false,
    enableClipboard: true,
});

const { t } = useTranslations('common.copy');

const isObject = computed(() => props.data !== null && typeof props.data === 'object' && !Array.isArray(props.data));
const isArray = computed(() => Array.isArray(props.data));
const isComplex = computed(() => isObject.value || isArray.value);

const isInitiallyCollapsed = computed(() => {
    if (typeof props.collapsed === 'boolean') {
        return props.collapsed;
    }
    if (typeof props.collapsed === 'number') {
        return props.depth >= props.collapsed;
    }
    return false;
});

const isExpanded = ref(!isInitiallyCollapsed.value);
const copied = ref(false);
let copyTimer: number | null = null;

const objectKeys = computed(() => {
    if (!isComplex.value || props.data === null || props.data === undefined) return [];
    return Object.keys(props.data);
});

const sizeLabel = computed(() => {
    if (isArray.value) {
        return `${props.data.length} item${props.data.length === 1 ? '' : 's'}`;
    }
    if (isObject.value) {
        return `${objectKeys.value.length} key${objectKeys.value.length === 1 ? '' : 's'}`;
    }
    return '';
});

const valueType = computed(() => {
    if (props.data === null) return 'null';
    if (isArray.value) return 'Array';
    return typeof props.data;
});

function toggleExpand() {
    isExpanded.value = !isExpanded.value;
}

async function copyNode(event: MouseEvent) {
    event.stopPropagation();
    try {
        const text = typeof props.data === 'string' ? props.data : JSON.stringify(props.data, null, 2);
        if (navigator.clipboard && window.isSecureContext) {
            await navigator.clipboard.writeText(text);
        } else {
            const textArea = document.createElement('textarea');
            textArea.value = text;
            textArea.style.position = 'fixed';
            textArea.style.left = '-999999px';
            document.body.appendChild(textArea);
            textArea.select();
            document.execCommand('copy');
            textArea.remove();
        }
        copied.value = true;
        toast.success(t('success') || 'Copied');
        if (copyTimer) window.clearTimeout(copyTimer);
        copyTimer = window.setTimeout(() => {
            copied.value = false;
        }, 1500);
    } catch {
        toast.error(t('failed') || 'Copy failed');
    }
}
</script>

<template>
    <div class="group/node relative font-mono text-[12px] leading-5 select-text">
        <!-- 复杂类型（对象/数组） -->
        <div v-if="isComplex" class="flex flex-col">
            <div
                class="flex items-center gap-1 cursor-pointer hover:bg-muted/30 rounded px-1 -mx-1 py-0.5 transition-colors w-fit"
                @click="toggleExpand"
            >
                <!-- 展开/折叠三角图标 -->
                <button
                    type="button"
                    class="size-3.5 flex items-center justify-center text-muted-foreground hover:text-foreground shrink-0"
                    aria-label="Toggle collapse"
                >
                    <LuChevronDown v-if="isExpanded" class="size-3" />
                    <LuChevronRight v-else class="size-3" />
                </button>

                <!-- 键名 -->
                <span v-if="nodeKey !== undefined" class="text-[#0550ae] dark:text-[#79c0ff] font-medium">
                    "{{ nodeKey }}":
                </span>

                <!-- 类型/大小提示 -->
                <span v-if="displayDataTypes" class="text-[10px] text-muted-foreground/70 mr-0.5">
                    {{ valueType }}
                </span>

                <!-- 未展开状态 -->
                <template v-if="!isExpanded">
                    <span class="text-muted-foreground">
                        {{ isArray ? '[' : '{' }}
                        <span class="text-[11px] text-muted-foreground/80 italic px-1">
                            {{ sizeLabel }}
                        </span>
                        {{ isArray ? ']' : '}' }}
                    </span>
                </template>

                <!-- 展开状态的左括号 -->
                <template v-else>
                    <span class="text-muted-foreground">{{ isArray ? '[' : '{' }}</span>
                    <span v-if="displayObjectSize" class="text-[10px] text-muted-foreground/60 ml-1">
                        // {{ sizeLabel }}
                    </span>
                </template>

                <!-- 复制按钮 -->
                <button
                    v-if="enableClipboard"
                    type="button"
                    @click="copyNode"
                    class="opacity-0 group-hover/node:opacity-100 transition-opacity ml-1 p-0.5 rounded text-muted-foreground hover:text-foreground"
                    title="Copy value"
                >
                    <LuCheck v-if="copied" class="size-3 text-emerald-500" />
                    <LuCopy v-else class="size-3" />
                </button>

                <span v-if="!isExpanded && !isLast" class="text-muted-foreground">,</span>
            </div>

            <!-- 子节点展开列表 -->
            <div v-if="isExpanded" class="pl-4 ml-1.5 border-l border-border/40 flex flex-col">
                <template v-if="isArray">
                    <JsonNode
                        v-for="(item, index) in data"
                        :key="index"
                        :data="item"
                        :node-key="index"
                        :depth="depth + 1"
                        :collapsed="collapsed"
                        :is-last="index === data.length - 1"
                        :display-data-types="displayDataTypes"
                        :display-object-size="displayObjectSize"
                        :enable-clipboard="enableClipboard"
                    />
                </template>
                <template v-else>
                    <JsonNode
                        v-for="(key, index) in objectKeys"
                        :key="key"
                        :data="data[key]"
                        :node-key="key"
                        :depth="depth + 1"
                        :collapsed="collapsed"
                        :is-last="index === objectKeys.length - 1"
                        :display-data-types="displayDataTypes"
                        :display-object-size="displayObjectSize"
                        :enable-clipboard="enableClipboard"
                    />
                </template>
            </div>

            <!-- 展开状态的右括号 -->
            <div v-if="isExpanded" class="text-muted-foreground pl-4">
                {{ isArray ? ']' : '}' }}<span v-if="!isLast">,</span>
            </div>
        </div>

        <!-- 基础类型（字符串/数字/布尔/null/undefined） -->
        <div v-else class="flex items-start gap-1 hover:bg-muted/30 rounded px-1 -mx-1 py-0.5 transition-colors">
            <span class="w-3.5 shrink-0" />
            <!-- 键名 -->
            <span v-if="nodeKey !== undefined" class="text-[#0550ae] dark:text-[#79c0ff] font-medium">
                "{{ nodeKey }}":
            </span>

            <!-- 类型标识 -->
            <span v-if="displayDataTypes" class="text-[10px] text-muted-foreground/70 mr-0.5">
                {{ valueType }}
            </span>

            <!-- 字符串值 -->
            <span
                v-if="typeof data === 'string'"
                class="text-[#0a3069] dark:text-[#7ee787] break-all whitespace-pre-wrap"
            >"{{ data }}"</span>

            <!-- 数值 -->
            <span
                v-else-if="typeof data === 'number'"
                class="text-[#005cc5] dark:text-[#d2a8ff]"
            >{{ data }}</span>

            <!-- 布尔值 -->
            <span
                v-else-if="typeof data === 'boolean'"
                class="text-[#cf222e] dark:text-[#ff7b72] font-semibold"
            >{{ data }}</span>

            <!-- Null / Undefined -->
            <span
                v-else-if="data === null || data === undefined"
                class="text-[#6e7781] dark:text-[#8b949e] italic"
            >{{ String(data) }}</span>

            <!-- 其他 -->
            <span v-else class="text-foreground">{{ String(data) }}</span>

            <!-- 逗号 -->
            <span v-if="!isLast" class="text-muted-foreground">,</span>

            <!-- 复制按钮 -->
            <button
                v-if="enableClipboard"
                type="button"
                @click="copyNode"
                class="opacity-0 group-hover/node:opacity-100 transition-opacity ml-1 p-0.5 rounded text-muted-foreground hover:text-foreground"
                title="Copy value"
            >
                <LuCheck v-if="copied" class="size-3 text-emerald-500" />
                <LuCopy v-else class="size-3" />
            </button>
        </div>
    </div>
</template>
