package com.bluetowel.audiovis;

import android.service.notification.NotificationListenerService;

/**
 * Does nothing itself. Android only tells an app what other apps are playing
 * (title, artist, cover and position, as shown in the notification shade's
 * media controls) if the app has a notification listener the user has
 * switched on, so this is that listener. It never looks at a notification;
 * MediaWatch asks for the media controls and nothing else.
 */
public class MediaListener extends NotificationListenerService {
}
