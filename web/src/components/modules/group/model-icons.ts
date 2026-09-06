import type { Component } from 'vue';
import {
    SiAnthropic,
    SiGooglegemini,
    SiMeta,
    SiNvidia,
    SiOpenai,
    SiPerplexity,
} from 'vue-icons-plus/si';
import { LuSparkles } from 'vue-icons-plus/lu';

export type ModelIconConfig = {
    keywords: string[];
    Icon: Component;
    className?: string;
    color: string;
};

const MODEL_ICON_PATTERNS: ModelIconConfig[] = [
    { keywords: ['gpt-', 'o1', 'o3', 'o4', 'chatgpt', 'text-embedding', 'dall-e', 'openai'], Icon: SiOpenai, className: 'brightness-0 dark:invert', color: '#10A37F' },
    { keywords: ['claude', 'anthropic'], Icon: SiAnthropic, color: '#D7765A' },
    { keywords: ['gemini', 'gemma', 'palm', 'google'], Icon: SiGooglegemini, color: '#4285F4' },
    { keywords: ['llama', 'meta-llama', 'meta'], Icon: SiMeta, color: '#0668E1' },
    { keywords: ['nvidia', 'nemotron'], Icon: SiNvidia, color: '#76B900' },
    { keywords: ['perplexity'], Icon: SiPerplexity, color: '#20B8CD' },
];

const DEFAULT_CONFIG = { Icon: LuSparkles, className: 'text-primary', color: '#10A37F' };

// getModelIcon 根据模型名称匹配对应的图标组件与品牌颜色。
export function getModelIcon(modelName: string): { Icon: Component; className?: string; color: string } {
    const lowerName = modelName.toLowerCase();
    for (const { keywords, Icon, className, color } of MODEL_ICON_PATTERNS) {
        if (keywords.some((keyword) => lowerName.includes(keyword))) {
            return { Icon, className, color };
        }
    }
    return DEFAULT_CONFIG;
}
