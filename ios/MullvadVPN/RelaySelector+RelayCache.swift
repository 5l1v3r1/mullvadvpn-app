//
//  RelaySelector+RelayCache.swift
//  MullvadVPN
//
//  Created by pronebird on 07/11/2019.
//  Copyright © 2019 Amagicom AB. All rights reserved.
//

import Combine
import Foundation

extension RelaySelector {

    static func loadedFromRelayCache() -> AnyPublisher<RelaySelector, RelayCacheError> {
        return RelayCache.withDefaultLocation().publisher
            .flatMap { $0.read() }
            .map { RelaySelector(relayList: $0.relayList) }
            .eraseToAnyPublisher()
    }

}
