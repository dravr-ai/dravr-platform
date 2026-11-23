// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Health Connect, read-only, for the device health sync: availability, the permission contract, changes-token reads per record type, day totals
// ABOUTME: Serializes records as barrage's DeviceSamples fields (raw constants, class names, values in fixed units); maps no meaning

package ai.dravr.health

import android.content.Context
import android.content.Intent
import android.util.Base64
import androidx.health.connect.client.HealthConnectClient
import androidx.health.connect.client.HealthConnectFeatures
import androidx.health.connect.client.PermissionController
import androidx.health.connect.client.changes.DeletionChange
import androidx.health.connect.client.changes.UpsertionChange
import androidx.health.connect.client.permission.HealthPermission
import androidx.health.connect.client.records.ActiveCaloriesBurnedRecord
import androidx.health.connect.client.records.DistanceRecord
import androidx.health.connect.client.records.ExerciseSessionRecord
import androidx.health.connect.client.records.HeartRateVariabilityRmssdRecord
import androidx.health.connect.client.records.Record
import androidx.health.connect.client.records.RestingHeartRateRecord
import androidx.health.connect.client.records.SleepSessionRecord
import androidx.health.connect.client.records.StepsRecord
import androidx.health.connect.client.records.WeightRecord
import androidx.health.connect.client.records.metadata.DataOrigin
import androidx.health.connect.client.records.metadata.Metadata
import androidx.health.connect.client.request.AggregateGroupByPeriodRequest
import androidx.health.connect.client.request.AggregateRequest
import androidx.health.connect.client.request.ChangesTokenRequest
import androidx.health.connect.client.request.ReadRecordsRequest
import androidx.health.connect.client.time.TimeRangeFilter
import expo.modules.kotlin.activityresult.AppContextActivityResultContract
import expo.modules.kotlin.activityresult.AppContextActivityResultLauncher
import expo.modules.kotlin.exception.CodedException
import expo.modules.kotlin.exception.Exceptions
import expo.modules.kotlin.functions.Coroutine
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import java.time.Instant
import java.time.LocalDate
import java.time.Period
import java.time.ZoneId
import java.time.ZoneOffset
import java.time.format.DateTimeParseException
import kotlin.reflect.KClass
import org.json.JSONException
import org.json.JSONObject

/** The record types this module reads, by the class name barrage reads them by. */
private val RECORD_TYPES: Map<String, KClass<out Record>> = mapOf(
  "ExerciseSessionRecord" to ExerciseSessionRecord::class,
  "SleepSessionRecord" to SleepSessionRecord::class,
  "StepsRecord" to StepsRecord::class,
  "ActiveCaloriesBurnedRecord" to ActiveCaloriesBurnedRecord::class,
  "DistanceRecord" to DistanceRecord::class,
  "RestingHeartRateRecord" to RestingHeartRateRecord::class,
  "HeartRateVariabilityRmssdRecord" to HeartRateVariabilityRmssdRecord::class,
  "WeightRecord" to WeightRecord::class,
)

/** Health Connect's own package; the client library keeps its constant for it internal. */
private const val HEALTH_CONNECT_PACKAGE = "com.google.android.apps.healthdata"

/** Whatever the day totals are filed under: the store itself, which merged them across apps. */
private val STORE_RECORDER = mapOf("name" to "Health Connect", "bundle" to HEALTH_CONNECT_PACKAGE)

class HealthConnectUnavailableException : CodedException("Health Connect is not available on this device")

class UnknownRecordTypeException(type: String) : CodedException("Not a record type this module reads: $type")

class BadArgumentException(what: String) : CodedException("Unreadable argument: $what")

/**
 * The permission sheet, as an Expo activity contract around Health Connect's
 * own: the input is the read permissions asked, the output those granted.
 */
private class PermissionContract : AppContextActivityResultContract<HashSet<String>, Set<String>> {
  private val contract = PermissionController.createRequestPermissionResultContract()

  override fun createIntent(context: Context, input: HashSet<String>): Intent = contract.createIntent(context, input)

  override fun parseResult(input: HashSet<String>, resultCode: Int, intent: Intent?): Set<String> =
    contract.parseResult(resultCode, intent)
}

/**
 * Where one record type's read stands, kept opaque by the JS side: the
 * changes token, and while the first read back to `since` is still paging,
 * that read's next page.
 */
private data class Cursor(val token: String, val backfillPage: String?, val backfilling: Boolean) {
  fun encode(): String {
    val json = JSONObject().put("token", token).put("backfilling", backfilling)
    if (backfillPage != null) json.put("page", backfillPage)
    return Base64.encodeToString(json.toString().toByteArray(), Base64.NO_WRAP)
  }

  companion object {
    fun decode(text: String): Cursor {
      val json = try {
        JSONObject(String(Base64.decode(text, Base64.NO_WRAP)))
      } catch (error: IllegalArgumentException) {
        throw BadArgumentException("anchor")
      } catch (error: JSONException) {
        throw BadArgumentException("anchor")
      }
      return Cursor(json.getString("token"), json.optString("page").ifEmpty { null }, json.optBoolean("backfilling"))
    }
  }
}

class DravrHealthModule : Module() {
  private var permissionLauncher: AppContextActivityResultLauncher<HashSet<String>, Set<String>>? = null

  private val context: Context
    get() = appContext.reactContext ?: throw Exceptions.ReactContextLost()

  private fun client(): HealthConnectClient {
    if (HealthConnectClient.getSdkStatus(context) != HealthConnectClient.SDK_AVAILABLE) {
      throw HealthConnectUnavailableException()
    }
    return HealthConnectClient.getOrCreate(context)
  }

  override fun definition() = ModuleDefinition {
    Name("DravrHealth")

    RegisterActivityContracts {
      permissionLauncher = registerForActivityResult(PermissionContract())
    }

    AsyncFunction("isAvailable") {
      HealthConnectClient.getSdkStatus(context) == HealthConnectClient.SDK_AVAILABLE
    }

    // Read permissions only, plus reading history past 30 days where the
    // store offers it, for the first read's 90 days. True when anything was
    // granted.
    AsyncFunction("requestAuthorization") Coroutine { identifiers: List<String> ->
      val client = client()
      val asked = identifiers.map { HealthPermission.getReadPermission(recordType(it)) }.toHashSet()
      val history = HealthConnectFeatures.FEATURE_READ_HEALTH_DATA_HISTORY
      if (client.features.getFeatureStatus(history) == HealthConnectFeatures.FEATURE_STATUS_AVAILABLE) {
        asked.add(HealthPermission.PERMISSION_READ_HEALTH_DATA_HISTORY)
      }
      val launcher = permissionLauncher ?: throw Exceptions.MissingActivity()
      val granted = client.permissionController.getGrantedPermissions().intersect(asked)
      if (granted.containsAll(asked)) return@Coroutine true
      launcher.launch(asked).any { it != HealthPermission.PERMISSION_READ_HEALTH_DATA_HISTORY }
    }

    // One page of one record type: the first read back to `since`, then the
    // changes since the token. An expired token reads back to `since` again.
    AsyncFunction("readChanges") Coroutine { type: String, anchor: String?, since: String, limit: Int ->
      readChanges(client(), recordType(type), anchor, instant(since), limit.coerceIn(1, 1000))
    }

    // Every sleep session's stages overlapping [from, to].
    AsyncFunction("readSleep") Coroutine { from: String, to: String ->
      val client = client()
      val sessions = mutableListOf<SleepSessionRecord>()
      var page: String? = null
      do {
        val response = client.readRecords(
          ReadRecordsRequest(
            recordType = SleepSessionRecord::class,
            timeRangeFilter = TimeRangeFilter.between(instant(from), instant(to)),
            pageSize = 1000,
            pageToken = page,
          ),
        )
        sessions.addAll(response.records)
        page = response.pageToken
      } while (page != null)
      sessions.flatMap(::stages)
    }

    // Every record of one quantity type in [from, to): a window re-read whole.
    AsyncFunction("readQuantities") Coroutine { type: String, from: String, to: String ->
      val client = client()
      val start = instant(from)
      val end = instant(to)
      val records = mutableListOf<Record>()
      var page: String? = null
      do {
        val response = client.readRecords(
          ReadRecordsRequest(
            recordType = recordType(type),
            timeRangeFilter = TimeRangeFilter.between(start, end),
            pageSize = 1000,
            pageToken = page,
          ),
        )
        records.addAll(response.records.filter { it.startInstant() >= start && it.startInstant() < end })
        page = response.pageToken
      } while (page != null)
      serialize(client, records).filter { it.first == "quantities" }.map { it.second }
    }

    // Each local day's steps and active energy, as Health Connect's aggregation merges its apps.
    AsyncFunction("readDays") Coroutine { dates: List<String> ->
      val days = dates.mapNotNull { runCatching { LocalDate.parse(it) }.getOrNull() }.toSortedSet()
      if (days.isEmpty()) return@Coroutine emptyList<Map<String, Any?>>()
      val zone = ZoneId.systemDefault()
      val groups = client().aggregateGroupByPeriod(
        AggregateGroupByPeriodRequest(
          metrics = setOf(StepsRecord.COUNT_TOTAL, ActiveCaloriesBurnedRecord.ACTIVE_CALORIES_TOTAL),
          timeRangeFilter = TimeRangeFilter.between(days.first().atStartOfDay(), days.last().plusDays(1).atStartOfDay()),
          timeRangeSlicer = Period.ofDays(1),
        ),
      )
      groups.mapNotNull { group ->
        val date = group.startTime.toLocalDate()
        val steps = group.result[StepsRecord.COUNT_TOTAL]
        val kcal = group.result[ActiveCaloriesBurnedRecord.ACTIVE_CALORIES_TOTAL]?.inKilocalories
        if (date !in days || (steps == null && kcal == null)) return@mapNotNull null
        mapOf(
          "date" to date.toString(),
          "utc_offset_seconds" to zone.rules.getOffset(date.atStartOfDay()).totalSeconds,
          "steps" to steps,
          "active_kcal" to kcal,
          "recorder" to STORE_RECORDER,
        )
      }
    }
  }

  private suspend fun readChanges(
    client: HealthConnectClient,
    type: KClass<out Record>,
    anchor: String?,
    since: Instant,
    limit: Int,
  ): Map<String, Any?> {
    // The token is taken before the first read, so nothing written while it
    // pages is missed; a record seen by both arrives twice and replaces itself.
    var cursor = anchor?.let(Cursor::decode)
      ?: Cursor(client.getChangesToken(ChangesTokenRequest(setOf(type))), null, true)
    val records = mutableListOf<Record>()
    val deleted = mutableListOf<String>()
    var more: Boolean
    if (cursor.backfilling) {
      val response = client.readRecords(
        ReadRecordsRequest(
          recordType = type,
          timeRangeFilter = TimeRangeFilter.after(since),
          pageSize = limit,
          pageToken = cursor.backfillPage,
        ),
      )
      records.addAll(response.records)
      cursor = Cursor(cursor.token, response.pageToken, response.pageToken != null)
      more = true
    } else {
      val response = client.getChanges(cursor.token)
      if (response.changesTokenExpired) {
        val fresh = Cursor(client.getChangesToken(ChangesTokenRequest(setOf(type))), null, true)
        return chunk(emptyList(), emptyList(), fresh, true)
      }
      for (change in response.changes) {
        when (change) {
          is UpsertionChange -> if (change.record.startInstant() >= since) records.add(change.record)
          is DeletionChange -> deleted.add(change.recordId)
        }
      }
      cursor = Cursor(response.nextChangesToken, null, false)
      more = response.hasMore
    }
    return chunk(serialize(client, records), deleted, cursor, more)
  }

  private fun chunk(rows: List<Pair<String, Map<String, Any?>>>, deleted: List<String>, cursor: Cursor, more: Boolean) =
    mapOf(
      "workouts" to rows.filter { it.first == "workouts" }.map { it.second },
      "categories" to rows.filter { it.first == "categories" }.map { it.second },
      "quantities" to rows.filter { it.first == "quantities" }.map { it.second },
      "states_of_mind" to emptyList<Map<String, Any?>>(),
      "deleted" to deleted,
      "anchor" to cursor.encode(),
      "more" to more,
    )

  /** Each record as the list it travels in and its fields. */
  private suspend fun serialize(client: HealthConnectClient, records: List<Record>): List<Pair<String, Map<String, Any?>>> =
    records.flatMap { record ->
      when (record) {
        is ExerciseSessionRecord -> listOf("workouts" to workout(client, record))
        is SleepSessionRecord -> stages(record).map { "categories" to it }
        is RestingHeartRateRecord ->
          listOf("quantities" to point(record.metadata, "RestingHeartRateRecord", record.beatsPerMinute.toDouble(), "bpm", record.time, record.zoneOffset))
        is HeartRateVariabilityRmssdRecord ->
          listOf("quantities" to point(record.metadata, "HeartRateVariabilityRmssdRecord", record.heartRateVariabilityMillis, "ms", record.time, record.zoneOffset))
        is WeightRecord ->
          listOf("quantities" to point(record.metadata, "WeightRecord", record.weight.inKilograms, "kg", record.time, record.zoneOffset))
        is StepsRecord ->
          listOf("quantities" to interval(record.metadata, "StepsRecord", record.count.toDouble(), "count", record.startTime, record.endTime, record.startZoneOffset))
        is ActiveCaloriesBurnedRecord ->
          listOf("quantities" to interval(record.metadata, "ActiveCaloriesBurnedRecord", record.energy.inKilocalories, "kcal", record.startTime, record.endTime, record.startZoneOffset))
        else -> emptyList()
      }
    }

  private suspend fun workout(client: HealthConnectClient, session: ExerciseSessionRecord): Map<String, Any?> {
    // The session's distance and energy are the recorder's own records over
    // its span, summed by the store.
    val totals = client.aggregate(
      AggregateRequest(
        metrics = setOf(DistanceRecord.DISTANCE_TOTAL, ActiveCaloriesBurnedRecord.ACTIVE_CALORIES_TOTAL),
        timeRangeFilter = TimeRangeFilter.between(session.startTime, session.endTime),
        dataOriginFilter = setOf(DataOrigin(session.metadata.dataOrigin.packageName)),
      ),
    )
    val meters = totals[DistanceRecord.DISTANCE_TOTAL]?.inMeters
    val kcal = totals[ActiveCaloriesBurnedRecord.ACTIVE_CALORIES_TOTAL]?.inKilocalories
    return mapOf(
      "id" to session.metadata.id,
      "activity_type" to session.exerciseType,
      "start" to session.startTime.toString(),
      "end" to session.endTime.toString(),
      "utc_offset_seconds" to offset(session.startZoneOffset, session.startTime),
      "duration_seconds" to (session.endTime.epochSecond - session.startTime.epochSecond).toDouble(),
      "distance" to meters?.let { mapOf("value" to it, "unit" to "m") },
      "energy" to kcal?.let { mapOf("value" to it, "unit" to "kcal") },
      "recorder" to recorder(session.metadata),
    )
  }

  /** A sleep session's stages, each a category sample `<id>-<n>`; a session with none is one unknown stage over it. */
  private fun stages(session: SleepSessionRecord): List<Map<String, Any?>> {
    val offset = offset(session.startZoneOffset, session.startTime)
    val spans = session.stages.ifEmpty {
      listOf(SleepSessionRecord.Stage(session.startTime, session.endTime, SleepSessionRecord.STAGE_TYPE_UNKNOWN))
    }
    return spans.mapIndexed { index, stage ->
      mapOf(
        "id" to "${session.metadata.id}-$index",
        "type" to "SleepSessionRecord",
        "value" to stage.stage,
        "start" to stage.startTime.toString(),
        "end" to stage.endTime.toString(),
        "utc_offset_seconds" to offset,
        "recorder" to recorder(session.metadata),
      )
    }
  }

  private fun point(metadata: Metadata, type: String, value: Double, unit: String, time: Instant, zone: ZoneOffset?) =
    interval(metadata, type, value, unit, time, time, zone)

  private fun interval(metadata: Metadata, type: String, value: Double, unit: String, start: Instant, end: Instant, zone: ZoneOffset?) =
    mapOf(
      "id" to metadata.id,
      "type" to type,
      "value" to value,
      "unit" to unit,
      "start" to start.toString(),
      "end" to end.toString(),
      "utc_offset_seconds" to offset(zone, start),
      "recorder" to recorder(metadata),
    )

  private fun recorder(metadata: Metadata): Map<String, Any?> = mapOf(
    "name" to metadata.dataOrigin.packageName,
    "bundle" to metadata.dataOrigin.packageName,
    "device_model" to metadata.device?.model,
  )

  /** The record's own offset, else the phone's at that instant. */
  private fun offset(zone: ZoneOffset?, at: Instant): Int =
    (zone ?: ZoneId.systemDefault().rules.getOffset(at)).totalSeconds

  private fun recordType(type: String): KClass<out Record> = RECORD_TYPES[type] ?: throw UnknownRecordTypeException(type)

  private fun instant(text: String): Instant =
    try {
      Instant.parse(text)
    } catch (error: DateTimeParseException) {
      throw BadArgumentException(text)
    }
}

/**
 * When a record starts: an interval record's start, an instantaneous record's
 * time. The library keeps IntervalRecord and InstantaneousRecord internal, so
 * each type in RECORD_TYPES is named. A type missing here fails its read loudly:
 * a fabricated start would filter every one of its records out unseen.
 */
private fun Record.startInstant(): Instant = when (this) {
  is ExerciseSessionRecord -> startTime
  is SleepSessionRecord -> startTime
  is StepsRecord -> startTime
  is ActiveCaloriesBurnedRecord -> startTime
  is DistanceRecord -> startTime
  is RestingHeartRateRecord -> time
  is HeartRateVariabilityRmssdRecord -> time
  is WeightRecord -> time
  else -> error("bug: startInstant has no start for ${this::class.simpleName}, a type RECORD_TYPES reads")
}
