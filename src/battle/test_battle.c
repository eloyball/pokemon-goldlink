#include "../../include/types.h"
#include "../../include/battle.h"
#include "../../include/message.h"
#include "../../include/test_battle.h"
#include "../../include/constants/file.h"

#ifdef DEBUG_BATTLE_SCENARIOS

BOOL LONG_CALL TestBattle_ExpectedMessageMatches(struct BattleSystem *bsys, MESSAGE_PARAM *actualMessage, const char *expectedMessage)
{
    if (bsys == NULL || actualMessage == NULL || expectedMessage == NULL || expectedMessage[0] == '\0') {
        return FALSE;
    }

    MsgData *msgData = NewMsgDataFromNarc(MSGDATA_LOAD_LAZY, ARC_MSG_DATA, TEST_BATTLE_MESSAGE_FILE_ID, HEAPID_DEFAULT);
    if (msgData == NULL) {
        return FALSE;
    }

    BattleSystem_BufferMessage(bsys, actualMessage);

    BattleMessage expandedMessage;
    expandedMessage.unk0 = actualMessage->command_code;
    expandedMessage.tag = actualMessage->msg_tag;
    expandedMessage.id = actualMessage->msg_id;
    expandedMessage.numDigits = actualMessage->numDigits;
    expandedMessage.battlerId = actualMessage->msg_client;
    for (int i = 0; i < 6; i++) {
        expandedMessage.param[i] = actualMessage->msg_para[i];
    }

    char actual[TEST_BATTLE_MESSAGE_LEN * 2];
    BattleMessage_ExpandPlaceholders(bsys, msgData, &expandedMessage, actual, sizeof(actual));
    DestroyMsgData(msgData);

    int i = 0;
    while (actual[i] != '\0' || expectedMessage[i] != '\0') {
        if (actual[i] != expectedMessage[i]) {
            return FALSE;
        }
        i++;
    }
    return TRUE;
}

void LONG_CALL BattleMessage_ExpandPlaceholders(struct BattleSystem *battleSystem, MsgData *data, BattleMessage *msg, char *dest, u32 destSize)
{
    String *str = NewString_ReadMsgData(data, msg->id);
    StringExpandPlaceholders(battleSystem->msgFormat, battleSystem->msgBuffer, str);
    String_Delete(str);

    if (dest == NULL || destSize == 0) {
        return;
    }

    u32 outIndex = 0;
    for (int i = 0; i < battleSystem->msgBuffer->size && outIndex + 1 < destSize; i++) {
        u32 code = battleSystem->msgBuffer->data[i];
        char character = '\0';

        if (code <= 0x0144) {
            character = (char)(code - 234);
        } else {
            switch (code) {
            case 0xFFFF:
                dest[outIndex] = '\0';
                return;
            case 0x01BE:
                character = '-';
                break;
            case 0x01DE:
            case 0x0001:
            case 0xE000:
            case 0x25BC:
            case 0x25BD:
                character = ' ';
                break;
            case 0x01AB:
                character = '!';
                break;
            case 0x01AD:
                character = ',';
                break;
            case 0x01AE:
                character = '.';
                break;
            case 0x01B3:
                character = '\'';
                break;
            default:
                character = (char)(code - 228);
                break;
            }
        }

        if (character != '\0') {
            dest[outIndex++] = character;
        }
    }
    dest[outIndex] = '\0';
}

#endif // DEBUG_BATTLE_SCENARIOS
