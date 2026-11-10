# \FirmsAreaRecordHandlerApi

All URIs are relative to *http://127.0.0.1:8070*

Method | HTTP request | Description
------------- | ------------- | -------------
[**search_firms_area_records_handler**](FirmsAreaRecordHandlerApi.md#search_firms_area_records_handler) | **POST** /api/v1/firms/area-records/search | 



## search_firms_area_records_handler

> models::PagedResponseFirmsAreaRecord search_firms_area_records_handler(search_firms_area_records_schema, page, limit, order_by, desc)


Search FIRMS area records by source, acquisition time, day/night, and location radius.

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**search_firms_area_records_schema** | [**SearchFirmsAreaRecordsSchema**](SearchFirmsAreaRecordsSchema.md) | FIRMS area record search filters | [required] |
**page** | Option<**i64**> |  |  |
**limit** | Option<**i64**> |  |  |
**order_by** | Option<**String**> |  |  |
**desc** | Option<**bool**> |  |  |

### Return type

[**models::PagedResponseFirmsAreaRecord**](PagedResponse_FirmsAreaRecord.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

