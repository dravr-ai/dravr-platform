// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: HealthKit, read-only, for the device health sync: authorization, anchored reads per type, sleep re-reads by window, day totals
// ABOUTME: Serializes samples as barrage's DeviceSamples fields (raw identifiers and enum numbers, values in fixed units); maps no meaning

import ExpoModulesCore
import HealthKit

/// The identifier the JS side names State of Mind by (`HKDataTypeIdentifierStateOfMind`'s value).
private let stateOfMindKey = "HKDataTypeStateOfMind"
private let workoutKey = "HKWorkoutTypeIdentifier"

/// The unit each quantity type is read in: the spelling barrage reads it by.
private let quantityUnits: [String: HKUnit] = [
  HKQuantityTypeIdentifier.restingHeartRate.rawValue: HKUnit.count().unitDivided(by: .minute()),
  HKQuantityTypeIdentifier.heartRateVariabilitySDNN.rawValue: HKUnit.secondUnit(with: .milli),
  HKQuantityTypeIdentifier.bodyMass.rawValue: HKUnit.gramUnit(with: .kilo),
  HKQuantityTypeIdentifier.bloodGlucose.rawValue: HKUnit(from: "mg/dL"),
  HKQuantityTypeIdentifier.basalBodyTemperature.rawValue: HKUnit.degreeCelsius(),
  HKQuantityTypeIdentifier.stepCount.rawValue: HKUnit.count(),
  HKQuantityTypeIdentifier.activeEnergyBurned.rawValue: HKUnit.kilocalorie(),
]

/// The distance types a workout's distance is read from, the first one it holds.
private let distanceTypes: [HKQuantityTypeIdentifier] = [
  .distanceWalkingRunning, .distanceCycling, .distanceSwimming, .distanceWheelchair, .distanceDownhillSnowSports,
]

final class HealthUnavailableException: Exception, @unchecked Sendable {
  override var reason: String { "Health data is not available on this device" }
}

final class UnknownTypeException: GenericException<String>, @unchecked Sendable {
  override var reason: String { "Not a type this module reads: \(param)" }
}

final class BadArgumentException: GenericException<String>, @unchecked Sendable {
  override var reason: String { "Unreadable argument: \(param)" }
}

public class DravrHealthModule: Module {
  private let store = HKHealthStore()

  public func definition() -> ModuleDefinition {
    Name("DravrHealth")

    AsyncFunction("isAvailable") { () -> Bool in
      HKHealthStore.isHealthDataAvailable()
    }

    // Read-only: nothing is ever shared back. HealthKit never says what the
    // athlete declined, so the answer is only that the prompt completed.
    AsyncFunction("requestAuthorization") { (identifiers: [String], promise: Promise) in
      guard HKHealthStore.isHealthDataAvailable() else {
        promise.reject(HealthUnavailableException())
        return
      }
      let types = Set(identifiers.compactMap { Self.objectType($0) as HKObjectType? })
      self.store.requestAuthorization(toShare: [], read: types) { success, error in
        if let error {
          promise.reject(error)
        } else {
          promise.resolve(success)
        }
      }
    }

    // One page of an anchored read of one type, from `since` on.
    AsyncFunction("readChanges") { (identifier: String, anchor: String?, since: String, limit: Int, promise: Promise) in
      if identifier == stateOfMindKey, Self.objectType(identifier) == nil {
        // State of Mind before iOS 18: nothing to read, the anchor stands.
        promise.resolve(Self.emptyChunk(anchor: anchor))
        return
      }
      guard let type = Self.objectType(identifier) else {
        promise.reject(UnknownTypeException(identifier))
        return
      }
      guard let start = Self.instant(since) else {
        promise.reject(BadArgumentException(since))
        return
      }
      let held: HKQueryAnchor?
      do {
        held = try anchor.map(Self.decodeAnchor)
      } catch {
        promise.reject(error)
        return
      }
      let predicate = HKQuery.predicateForSamples(withStart: start, end: nil, options: [])
      let query = HKAnchoredObjectQuery(
        type: type, predicate: predicate, anchor: held, limit: max(1, limit)
      ) { _, samples, deleted, newAnchor, error in
        if let error {
          promise.reject(error)
          return
        }
        var chunk = Self.emptyChunk(anchor: nil)
        let samples = samples ?? []
        let deleted = deleted ?? []
        Self.collect(samples, into: &chunk)
        chunk["deleted"] = deleted.map { $0.uuid.uuidString }
        do {
          if let next = try newAnchor.map(Self.encodeAnchor) ?? anchor { chunk["anchor"] = next }
        } catch {
          promise.reject(error)
          return
        }
        chunk["more"] = samples.count + deleted.count >= max(1, limit)
        promise.resolve(chunk)
      }
      self.store.execute(query)
    }

    // Every sleep sample overlapping [from, to].
    AsyncFunction("readSleep") { (from: String, to: String, promise: Promise) in
      guard let start = Self.instant(from), let end = Self.instant(to) else {
        promise.reject(BadArgumentException("\(from)–\(to)"))
        return
      }
      guard let type = HKObjectType.categoryType(forIdentifier: .sleepAnalysis) else {
        promise.reject(UnknownTypeException(HKCategoryTypeIdentifier.sleepAnalysis.rawValue))
        return
      }
      let predicate = HKQuery.predicateForSamples(withStart: start, end: end, options: [])
      let sort = NSSortDescriptor(key: HKSampleSortIdentifierStartDate, ascending: true)
      let query = HKSampleQuery(
        sampleType: type, predicate: predicate, limit: HKObjectQueryNoLimit, sortDescriptors: [sort]
      ) { _, samples, error in
        if let error {
          promise.reject(error)
          return
        }
        let categories = (samples ?? []).compactMap { $0 as? HKCategorySample }.map(Self.category)
        promise.resolve(categories)
      }
      self.store.execute(query)
    }

    // Every sample of one quantity type starting in [from, to): a window re-read whole.
    AsyncFunction("readQuantities") { (identifier: String, from: String, to: String, promise: Promise) in
      guard identifier.hasPrefix("HKQuantityTypeIdentifier"),
            let type = HKObjectType.quantityType(forIdentifier: HKQuantityTypeIdentifier(rawValue: identifier)) else {
        promise.reject(UnknownTypeException(identifier))
        return
      }
      guard let start = Self.instant(from), let end = Self.instant(to) else {
        promise.reject(BadArgumentException("\(from)–\(to)"))
        return
      }
      let predicate = HKQuery.predicateForSamples(withStart: start, end: end, options: .strictStartDate)
      let query = HKSampleQuery(
        sampleType: type, predicate: predicate, limit: HKObjectQueryNoLimit, sortDescriptors: nil
      ) { _, samples, error in
        if let error {
          promise.reject(error)
          return
        }
        let rows = (samples ?? []).compactMap { $0 as? HKQuantitySample }.compactMap(Self.quantity)
          .filter { ($0["start"] as? String).flatMap(Self.instant).map { $0 < end } ?? false }
        promise.resolve(rows)
      }
      self.store.execute(query)
    }

    // Each local day's steps and active energy, as HealthKit's statistics merge its sources.
    AsyncFunction("readDays") { (dates: [String], promise: Promise) in
      let calendar = Calendar.current
      let midnights = dates.compactMap { Self.localMidnight($0, calendar) }
      guard let first = midnights.min(), let last = midnights.max(),
            let end = calendar.date(byAdding: .day, value: 1, to: last) else {
        promise.resolve([[String: Any]]())
        return
      }
      var steps: [Date: Double] = [:]
      var energy: [Date: Double] = [:]
      var failure: Error?
      let group = DispatchGroup()
      let lock = NSLock()
      for (identifier, unit) in [(HKQuantityTypeIdentifier.stepCount, HKUnit.count()),
                                 (.activeEnergyBurned, HKUnit.kilocalorie())] {
        guard let type = HKQuantityType.quantityType(forIdentifier: identifier) else { continue }
        group.enter()
        let query = HKStatisticsCollectionQuery(
          quantityType: type,
          quantitySamplePredicate: HKQuery.predicateForSamples(withStart: first, end: end, options: []),
          options: .cumulativeSum,
          anchorDate: first,
          intervalComponents: DateComponents(day: 1)
        )
        query.initialResultsHandler = { _, collection, error in
          lock.lock()
          if let error {
            failure = error
          } else if let collection {
            for midnight in midnights {
              if let sum = collection.statistics(for: midnight)?.sumQuantity()?.doubleValue(for: unit) {
                if identifier == .stepCount { steps[midnight] = sum } else { energy[midnight] = sum }
              }
            }
          }
          lock.unlock()
          group.leave()
        }
        self.store.execute(query)
      }
      group.notify(queue: .global()) {
        if let failure {
          promise.reject(failure)
          return
        }
        var days: [[String: Any]] = []
        for midnight in Set(midnights).sorted() {
          let count = steps[midnight]
          let kcal = energy[midnight]
          if count == nil && kcal == nil { continue }
          var day: [String: Any] = [
            "date": Self.civilDate(midnight, calendar),
            "utc_offset_seconds": calendar.timeZone.secondsFromGMT(for: midnight),
            "recorder": ["name": "Health", "bundle": "com.apple.Health"],
          ]
          if let count { day["steps"] = Int(count.rounded()) }
          if let kcal { day["active_kcal"] = kcal }
          days.append(day)
        }
        promise.resolve(days)
      }
    }
  }

  // MARK: - Types

  private static func objectType(_ identifier: String) -> HKSampleType? {
    if identifier == workoutKey {
      return HKObjectType.workoutType()
    }
    if identifier == stateOfMindKey {
      if #available(iOS 18.0, *) {
        return HKObjectType.stateOfMindType()
      }
      return nil
    }
    if identifier.hasPrefix("HKQuantityTypeIdentifier") {
      return HKObjectType.quantityType(forIdentifier: HKQuantityTypeIdentifier(rawValue: identifier))
    }
    if identifier.hasPrefix("HKCategoryTypeIdentifier") {
      return HKObjectType.categoryType(forIdentifier: HKCategoryTypeIdentifier(rawValue: identifier))
    }
    return nil
  }

  // MARK: - Serialization

  /// A page holding nothing; `anchor` is left out when there is none, which JS reads as no anchor.
  private static func emptyChunk(anchor: String?) -> [String: Any] {
    var chunk: [String: Any] = [
      "workouts": [[String: Any]](),
      "categories": [[String: Any]](),
      "quantities": [[String: Any]](),
      "states_of_mind": [[String: Any]](),
      "deleted": [String](),
      "more": false,
    ]
    if let anchor { chunk["anchor"] = anchor }
    return chunk
  }

  private static func collect(_ samples: [HKSample], into chunk: inout [String: Any]) {
    var workouts: [[String: Any]] = []
    var categories: [[String: Any]] = []
    var quantities: [[String: Any]] = []
    var moods: [[String: Any]] = []
    for sample in samples {
      if let workout = sample as? HKWorkout {
        workouts.append(Self.workout(workout))
      } else if let category = sample as? HKCategorySample {
        categories.append(Self.category(category))
      } else if let quantity = sample as? HKQuantitySample, let row = Self.quantity(quantity) {
        quantities.append(row)
      } else if #available(iOS 18.0, *), let mood = sample as? HKStateOfMind {
        moods.append(Self.stateOfMind(mood))
      }
    }
    chunk["workouts"] = workouts
    chunk["categories"] = categories
    chunk["quantities"] = quantities
    chunk["states_of_mind"] = moods
  }

  private static func workout(_ workout: HKWorkout) -> [String: Any] {
    var row = Self.span(workout)
    row["activity_type"] = Int(workout.workoutActivityType.rawValue)
    row["duration_seconds"] = workout.duration
    if let meters = distanceTypes.lazy.compactMap({ Self.sum(workout, $0, .meter()) }).first {
      row["distance"] = ["value": meters, "unit": "m"]
    }
    if let kcal = Self.sum(workout, .activeEnergyBurned, .kilocalorie()) {
      row["energy"] = ["value": kcal, "unit": "kcal"]
    }
    if let indoor = workout.metadata?[HKMetadataKeyIndoorWorkout] as? Bool {
      row["indoor"] = indoor
    }
    return row
  }

  private static func sum(_ workout: HKWorkout, _ identifier: HKQuantityTypeIdentifier, _ unit: HKUnit) -> Double? {
    guard let type = HKQuantityType.quantityType(forIdentifier: identifier) else { return nil }
    return workout.statistics(for: type)?.sumQuantity()?.doubleValue(for: unit)
  }

  private static func category(_ sample: HKCategorySample) -> [String: Any] {
    var row = Self.span(sample)
    row["type"] = sample.categoryType.identifier
    row["value"] = sample.value
    if let start = sample.metadata?[HKMetadataKeyMenstrualCycleStart] as? Bool {
      row["cycle_start"] = start
    }
    if let used = sample.metadata?[HKMetadataKeySexualActivityProtectionUsed] as? Bool {
      row["protection_used"] = used
    }
    return row
  }

  private static func quantity(_ sample: HKQuantitySample) -> [String: Any]? {
    let identifier = sample.quantityType.identifier
    guard let unit = quantityUnits[identifier], sample.quantity.is(compatibleWith: unit) else { return nil }
    var row = Self.span(sample)
    row["type"] = identifier
    row["value"] = sample.quantity.doubleValue(for: unit)
    row["unit"] = unit.unitString
    return row
  }

  @available(iOS 18.0, *)
  private static func stateOfMind(_ sample: HKStateOfMind) -> [String: Any] {
    [
      "id": sample.uuid.uuidString,
      "kind": sample.kind.rawValue,
      "valence": sample.valence,
      "classification": sample.valenceClassification.rawValue,
      "start": Self.isoString(sample.startDate),
      "utc_offset_seconds": Self.offset(sample),
      "recorder": Self.recorder(sample),
    ]
  }

  /// The fields every sample with a span carries.
  private static func span(_ sample: HKSample) -> [String: Any] {
    [
      "id": sample.uuid.uuidString,
      "start": Self.isoString(sample.startDate),
      "end": Self.isoString(sample.endDate),
      "utc_offset_seconds": Self.offset(sample),
      "recorder": Self.recorder(sample),
    ]
  }

  private static func recorder(_ sample: HKSample) -> [String: Any] {
    let source = sample.sourceRevision.source
    var recorder: [String: Any] = ["name": source.name, "bundle": source.bundleIdentifier]
    if let model = sample.sourceRevision.productType ?? sample.device?.model {
      recorder["device_model"] = model
    }
    return recorder
  }

  /// The offset of the clock the sample was recorded on: its time zone metadata, else the phone's.
  private static func offset(_ sample: HKSample) -> Int {
    let zone = (sample.metadata?[HKMetadataKeyTimeZone] as? String).flatMap(TimeZone.init(identifier:))
    return (zone ?? TimeZone.current).secondsFromGMT(for: sample.startDate)
  }

  // MARK: - Instants and anchors

  // ISO8601DateFormatter is thread-safe, so one of each serves every query's handler queue.
  private static let fractional: ISO8601DateFormatter = {
    let formatter = ISO8601DateFormatter()
    formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    return formatter
  }()

  private static let whole: ISO8601DateFormatter = {
    let formatter = ISO8601DateFormatter()
    formatter.formatOptions = [.withInternetDateTime]
    return formatter
  }()

  private static func isoString(_ date: Date) -> String {
    fractional.string(from: date)
  }

  private static func instant(_ text: String) -> Date? {
    fractional.date(from: text) ?? whole.date(from: text)
  }

  private static func localMidnight(_ civil: String, _ calendar: Calendar) -> Date? {
    let parts = civil.split(separator: "-").compactMap { Int($0) }
    guard parts.count == 3 else { return nil }
    return calendar.date(from: DateComponents(year: parts[0], month: parts[1], day: parts[2]))
  }

  private static func civilDate(_ date: Date, _ calendar: Calendar) -> String {
    let parts = calendar.dateComponents([.year, .month, .day], from: date)
    return String(format: "%04d-%02d-%02d", parts.year ?? 0, parts.month ?? 0, parts.day ?? 0)
  }

  private static func encodeAnchor(_ anchor: HKQueryAnchor) throws -> String {
    try NSKeyedArchiver.archivedData(withRootObject: anchor, requiringSecureCoding: true).base64EncodedString()
  }

  private static func decodeAnchor(_ text: String) throws -> HKQueryAnchor {
    guard let data = Data(base64Encoded: text),
          let anchor = try NSKeyedUnarchiver.unarchivedObject(ofClass: HKQueryAnchor.self, from: data) else {
      throw BadArgumentException("anchor")
    }
    return anchor
  }
}
