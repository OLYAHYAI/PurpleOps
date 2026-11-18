/**
 * @file cmd_serial.h
 * @brief Démarre la tâche d'interface de commande UART (voir cmd_serial.c).
 */
#ifndef CMD_SERIAL_H
#define CMD_SERIAL_H

/** Lance la tâche FreeRTOS qui lit les commandes sur UART0 (port USB-série). */
void cmd_serial_start(void);

#endif
