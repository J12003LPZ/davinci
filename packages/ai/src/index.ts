import type { Message, ToolCall } from '@pi/core';

export type StopReason = 'stop' | 'length' | 'toolUse' | 'error' | 'aborted';

export interface ToolDefinition {
  name: string;
  description: string;
  parameters: unknown;
}

export interface CompletionResponse {
  content: string;
  toolCalls?: ToolCall[];
  stopReason: StopReason;
}

export type AssistantMessageEvent =
  | { type: 'start'; messageId: string }
  | { type: 'text_delta'; messageId: string; delta: string }
  | { type: 'done'; stopReason: StopReason }
  | { type: 'error'; message: string };

export function lastUserText(messages: Message[]): string {
  for (let i = messages.length - 1; i >= 0; i--) {
    if (messages[i].role === 'user') return messages[i].content;
  }
  return '';
}
