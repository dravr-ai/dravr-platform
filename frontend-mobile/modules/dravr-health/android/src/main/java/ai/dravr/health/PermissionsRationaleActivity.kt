// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Health Connect's permissions-rationale screen — opens Dravr's privacy policy in the browser and closes, showing nothing of its own
// ABOUTME: The policy's address is the manifest meta-data the withDravrHealth config plugin writes from app.config.js

package ai.dravr.health

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.util.Log

/** The application meta-data key holding the privacy policy's address. */
const val PRIVACY_POLICY_URL_KEY = "ai.dravr.health.PRIVACY_POLICY_URL"

class PermissionsRationaleActivity : Activity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    val url = privacyPolicyUrl()
    if (url == null) {
      Log.w(TAG, "No $PRIVACY_POLICY_URL_KEY meta-data: the config plugin did not run")
    } else {
      try {
        startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
      } catch (error: ActivityNotFoundException) {
        Log.w(TAG, "No browser to open the privacy policy", error)
      }
    }
    // Theme.NoDisplay: the activity must finish before it would resume.
    finish()
  }

  private fun privacyPolicyUrl(): String? {
    val info = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
      packageManager.getApplicationInfo(packageName, PackageManager.ApplicationInfoFlags.of(PackageManager.GET_META_DATA.toLong()))
    } else {
      @Suppress("DEPRECATION")
      packageManager.getApplicationInfo(packageName, PackageManager.GET_META_DATA)
    }
    return info.metaData?.getString(PRIVACY_POLICY_URL_KEY)
  }

  private companion object {
    const val TAG = "DravrHealth"
  }
}
