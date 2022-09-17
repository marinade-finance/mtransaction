// package: validator
// file: mtransaction.proto

/* tslint:disable */
/* eslint-disable */

import * as grpc from "@grpc/grpc-js";
import * as mtransaction_pb from "./mtransaction_pb";

interface IMTransactionService extends grpc.ServiceDefinition<grpc.UntypedServiceImplementation> {
    echo: IMTransactionService_IEcho;
    echoStream: IMTransactionService_IEchoStream;
}

interface IMTransactionService_IEcho extends grpc.MethodDefinition<mtransaction_pb.EchoRequest, mtransaction_pb.EchoResponse> {
    path: "/validator.MTransaction/Echo";
    requestStream: false;
    responseStream: false;
    requestSerialize: grpc.serialize<mtransaction_pb.EchoRequest>;
    requestDeserialize: grpc.deserialize<mtransaction_pb.EchoRequest>;
    responseSerialize: grpc.serialize<mtransaction_pb.EchoResponse>;
    responseDeserialize: grpc.deserialize<mtransaction_pb.EchoResponse>;
}
interface IMTransactionService_IEchoStream extends grpc.MethodDefinition<mtransaction_pb.EchoRequest, mtransaction_pb.EchoResponse> {
    path: "/validator.MTransaction/EchoStream";
    requestStream: false;
    responseStream: true;
    requestSerialize: grpc.serialize<mtransaction_pb.EchoRequest>;
    requestDeserialize: grpc.deserialize<mtransaction_pb.EchoRequest>;
    responseSerialize: grpc.serialize<mtransaction_pb.EchoResponse>;
    responseDeserialize: grpc.deserialize<mtransaction_pb.EchoResponse>;
}

export const MTransactionService: IMTransactionService;

export interface IMTransactionServer extends grpc.UntypedServiceImplementation {
    echo: grpc.handleUnaryCall<mtransaction_pb.EchoRequest, mtransaction_pb.EchoResponse>;
    echoStream: grpc.handleServerStreamingCall<mtransaction_pb.EchoRequest, mtransaction_pb.EchoResponse>;
}

export interface IMTransactionClient {
    echo(request: mtransaction_pb.EchoRequest, callback: (error: grpc.ServiceError | null, response: mtransaction_pb.EchoResponse) => void): grpc.ClientUnaryCall;
    echo(request: mtransaction_pb.EchoRequest, metadata: grpc.Metadata, callback: (error: grpc.ServiceError | null, response: mtransaction_pb.EchoResponse) => void): grpc.ClientUnaryCall;
    echo(request: mtransaction_pb.EchoRequest, metadata: grpc.Metadata, options: Partial<grpc.CallOptions>, callback: (error: grpc.ServiceError | null, response: mtransaction_pb.EchoResponse) => void): grpc.ClientUnaryCall;
    echoStream(request: mtransaction_pb.EchoRequest, options?: Partial<grpc.CallOptions>): grpc.ClientReadableStream<mtransaction_pb.EchoResponse>;
    echoStream(request: mtransaction_pb.EchoRequest, metadata?: grpc.Metadata, options?: Partial<grpc.CallOptions>): grpc.ClientReadableStream<mtransaction_pb.EchoResponse>;
}

export class MTransactionClient extends grpc.Client implements IMTransactionClient {
    constructor(address: string, credentials: grpc.ChannelCredentials, options?: Partial<grpc.ClientOptions>);
    public echo(request: mtransaction_pb.EchoRequest, callback: (error: grpc.ServiceError | null, response: mtransaction_pb.EchoResponse) => void): grpc.ClientUnaryCall;
    public echo(request: mtransaction_pb.EchoRequest, metadata: grpc.Metadata, callback: (error: grpc.ServiceError | null, response: mtransaction_pb.EchoResponse) => void): grpc.ClientUnaryCall;
    public echo(request: mtransaction_pb.EchoRequest, metadata: grpc.Metadata, options: Partial<grpc.CallOptions>, callback: (error: grpc.ServiceError | null, response: mtransaction_pb.EchoResponse) => void): grpc.ClientUnaryCall;
    public echoStream(request: mtransaction_pb.EchoRequest, options?: Partial<grpc.CallOptions>): grpc.ClientReadableStream<mtransaction_pb.EchoResponse>;
    public echoStream(request: mtransaction_pb.EchoRequest, metadata?: grpc.Metadata, options?: Partial<grpc.CallOptions>): grpc.ClientReadableStream<mtransaction_pb.EchoResponse>;
}
